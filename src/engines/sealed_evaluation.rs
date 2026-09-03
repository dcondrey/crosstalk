//! Signed, replay-resistant evaluation across a trust boundary.
//!
//! A sealed worker owns hidden test material and a signing key. Clients submit
//! complete, expiring requests and accept results only when the receipt is
//! signed by a pinned worker identity. The transport remains replaceable.

use crate::engines::objective_evaluation::{
    CandidateArtifact, EvaluationError, EvaluationSpec, ObjectiveEvaluation, ObjectiveEvaluator,
};
use async_trait::async_trait;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

pub const SEALED_REQUEST_SCHEMA: &str = "crosstalk.sealed-evaluation-request.v1";
pub const SEALED_RECEIPT_SCHEMA: &str = "crosstalk.sealed-evaluation-receipt.v1";
const MAX_REQUEST_LIFETIME_SECS: u64 = 24 * 60 * 60;
const MAX_CLOCK_SKEW_SECS: u64 = 60;
const MAX_REPLAY_ENTRIES: usize = 100_000;
const MAX_ID_BYTES: usize = 4_096;
const MAX_REQUEST_WIRE_BYTES: usize = 192 * 1024 * 1024;
const DEFAULT_MAX_RECEIPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_WORKER_STDERR_BYTES: usize = 64 * 1024;

const ATTESTATION_ENV_KEYS: [&str; 10] = [
    "sealed_attestation_schema",
    "sealed_worker_id",
    "sealed_worker_key_sha256",
    "sealed_verifying_key_hex",
    "sealed_request_sha256",
    "sealed_evaluation_sha256",
    "sealed_receipt_issued_at",
    "sealed_receipt_signature",
    "sealed_receipt_sha256",
    "sealed_test_commitment_sha256",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedEvaluationRequest {
    pub schema: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub test_commitment_sha256: String,
    pub evaluation: EvaluationSpec,
    pub candidate: CandidateArtifact,
    /// Random 256-bit nonce encoded as lowercase hexadecimal.
    pub nonce: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl SealedEvaluationRequest {
    pub fn validate_at(&self, now: u64) -> Result<(), EvaluationError> {
        if self.schema != SEALED_REQUEST_SCHEMA {
            return Err(EvaluationError::InvalidSpec(format!(
                "unsupported sealed request schema: {}",
                self.schema
            )));
        }
        if self.evaluator_id.trim().is_empty()
            || self.evaluator_version.trim().is_empty()
            || self.evaluator_id.len() > MAX_ID_BYTES
            || self.evaluator_version.len() > MAX_ID_BYTES
        {
            return Err(EvaluationError::InvalidSpec(
                "sealed evaluator identity is empty or too large".into(),
            ));
        }
        self.evaluation.validate()?;
        self.candidate.validate()?;
        validate_sha256(&self.test_commitment_sha256, "hidden-test commitment")?;
        decode_hex::<32>(&self.nonce, "request nonce")?;
        if self.expires_at <= self.issued_at {
            return Err(EvaluationError::InvalidSpec(
                "sealed request expiry must follow issuance".into(),
            ));
        }
        if self.expires_at.saturating_sub(self.issued_at) > MAX_REQUEST_LIFETIME_SECS {
            return Err(EvaluationError::InvalidSpec(
                "sealed request lifetime exceeds 24 hours".into(),
            ));
        }
        if self.issued_at > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
            return Err(EvaluationError::InvalidSpec(
                "sealed request issuance is too far in the future".into(),
            ));
        }
        if now > self.expires_at.saturating_add(MAX_CLOCK_SKEW_SECS) {
            return Err(EvaluationError::Evaluator(
                "sealed evaluation request expired".into(),
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, EvaluationError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        Ok(sha256(&bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedReceiptPayload {
    pub schema: String,
    pub request_sha256: String,
    pub worker_id: String,
    pub worker_key_sha256: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub test_commitment_sha256: String,
    pub evaluation_sha256: String,
    pub issued_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedEvaluationReceipt {
    pub payload: SealedReceiptPayload,
    pub verifying_key_hex: String,
    pub signature_hex: String,
    pub evaluation: ObjectiveEvaluation,
}

impl SignedEvaluationReceipt {
    pub fn sha256(&self) -> Result<String, EvaluationError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        Ok(sha256(&bytes))
    }
}

#[async_trait]
pub trait SealedEvaluationTransport: Send + Sync {
    async fn submit(
        &self,
        request: SealedEvaluationRequest,
    ) -> Result<SignedEvaluationReceipt, EvaluationError>;
}

/// Worker-side endpoint. Hidden cases remain encapsulated by the evaluator;
/// only a signed objective result crosses the transport boundary.
pub struct SealedEvaluatorWorker {
    worker_id: String,
    evaluator: Arc<dyn ObjectiveEvaluator>,
    signing_key: SigningKey,
    replay_cache: Mutex<BTreeMap<String, u64>>,
}

impl SealedEvaluatorWorker {
    pub fn new_random(
        worker_id: impl Into<String>,
        evaluator: Arc<dyn ObjectiveEvaluator>,
    ) -> Result<Self, EvaluationError> {
        let seed = Zeroizing::new(rand::rng().random::<[u8; 32]>());
        Self::from_seed(worker_id, evaluator, *seed)
    }

    pub fn from_seed(
        worker_id: impl Into<String>,
        evaluator: Arc<dyn ObjectiveEvaluator>,
        seed: [u8; 32],
    ) -> Result<Self, EvaluationError> {
        let worker_id = worker_id.into();
        if worker_id.trim().is_empty() || worker_id.len() > MAX_ID_BYTES {
            return Err(EvaluationError::InvalidSpec(
                "sealed worker id is empty or too large".into(),
            ));
        }
        let commitment = evaluator.test_commitment_sha256().ok_or_else(|| {
            EvaluationError::InvalidSpec(
                "sealed workers require an evaluator-owned test commitment".into(),
            )
        })?;
        validate_sha256(commitment, "hidden-test commitment")?;
        if evaluator.attestation_key_sha256().is_some() {
            return Err(EvaluationError::InvalidSpec(
                "a sealed worker cannot wrap an already-attested evaluator".into(),
            ));
        }
        Ok(Self {
            worker_id,
            evaluator,
            signing_key: SigningKey::from_bytes(&seed),
            replay_cache: Mutex::new(BTreeMap::new()),
        })
    }

    #[must_use]
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    #[must_use]
    pub fn key_fingerprint_sha256(&self) -> String {
        sha256(&self.verifying_key().to_bytes())
    }

    pub async fn handle(
        &self,
        request: SealedEvaluationRequest,
    ) -> Result<SignedEvaluationReceipt, EvaluationError> {
        let started_at = now();
        request.validate_at(started_at)?;
        if request.evaluator_id != self.evaluator.id()
            || request.evaluator_version != self.evaluator.version()
        {
            return Err(EvaluationError::InvalidSpec(
                "sealed request evaluator identity does not match worker".into(),
            ));
        }
        let worker_commitment = self.evaluator.test_commitment_sha256().ok_or_else(|| {
            EvaluationError::InvalidSpec("worker evaluator lost its test commitment".into())
        })?;
        if !request
            .test_commitment_sha256
            .eq_ignore_ascii_case(worker_commitment)
        {
            return Err(EvaluationError::InvalidSpec(
                "sealed request hidden-test commitment does not match worker".into(),
            ));
        }

        let request_sha256 = request.sha256()?;
        {
            let mut cache = self.replay_cache.lock().map_err(|_| {
                EvaluationError::Evaluator("sealed replay cache lock was poisoned".into())
            })?;
            cache.retain(|_, expires_at| {
                expires_at.saturating_add(MAX_CLOCK_SKEW_SECS) >= started_at
            });
            if cache.contains_key(&request_sha256) {
                return Err(EvaluationError::Evaluator(
                    "sealed evaluation request replay rejected".into(),
                ));
            }
            if cache.len() >= MAX_REPLAY_ENTRIES {
                return Err(EvaluationError::Evaluator(
                    "sealed replay cache is full; refusing unevictable request".into(),
                ));
            }
            // Consume before execution. A failed run cannot be replayed to
            // probe nondeterminism or hidden worker state.
            cache.insert(request_sha256.clone(), request.expires_at);
        }

        let evaluation = tokio::time::timeout(
            std::time::Duration::from_secs(request.evaluation.timeout_secs),
            self.evaluator
                .evaluate(&request.evaluation, &request.candidate),
        )
        .await
        .map_err(|_| EvaluationError::Evaluator("sealed worker timed out".into()))??;
        evaluation.validate_against(&request.evaluation)?;
        if evaluation.evaluator_id != self.evaluator.id()
            || evaluation.evaluator_version != self.evaluator.version()
            || evaluation.candidate_id != request.candidate.id
            || evaluation.candidate_sha256 != request.candidate.sha256()
        {
            return Err(EvaluationError::MalformedResult(
                "sealed worker evaluator returned mismatched provenance".into(),
            ));
        }
        if evaluation
            .environment
            .keys()
            .any(|key| key.starts_with("sealed_"))
        {
            return Err(EvaluationError::MalformedResult(
                "underlying evaluator used the reserved sealed attestation namespace".into(),
            ));
        }

        let evaluation_sha256 = evaluation_sha256(&evaluation)?;
        let verifying_key = self.verifying_key();
        let worker_key_sha256 = sha256(&verifying_key.to_bytes());
        let payload = SealedReceiptPayload {
            schema: SEALED_RECEIPT_SCHEMA.into(),
            request_sha256,
            worker_id: self.worker_id.clone(),
            worker_key_sha256,
            evaluator_id: self.evaluator.id().into(),
            evaluator_version: self.evaluator.version().into(),
            test_commitment_sha256: worker_commitment.to_ascii_lowercase(),
            evaluation_sha256,
            issued_at: now(),
        };
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        let signature = self.signing_key.sign(&payload_bytes);
        Ok(SignedEvaluationReceipt {
            payload,
            verifying_key_hex: encode_hex(&verifying_key.to_bytes()),
            signature_hex: encode_hex(&signature.to_bytes()),
            evaluation,
        })
    }
}

pub struct InProcessSealedTransport {
    worker: Arc<SealedEvaluatorWorker>,
}

impl InProcessSealedTransport {
    #[must_use]
    pub fn new(worker: Arc<SealedEvaluatorWorker>) -> Self {
        Self { worker }
    }
}

#[async_trait]
impl SealedEvaluationTransport for InProcessSealedTransport {
    async fn submit(
        &self,
        request: SealedEvaluationRequest,
    ) -> Result<SignedEvaluationReceipt, EvaluationError> {
        self.worker.handle(request).await
    }
}

/// JSON-over-stdio transport for an isolated process or an SSH command. The
/// program is executed directly without a shell, so metacharacters in
/// arguments are not interpreted locally.
pub struct ProcessSealedTransport {
    program: PathBuf,
    args: Vec<OsString>,
    timeout_secs: u64,
    max_receipt_bytes: usize,
}

impl ProcessSealedTransport {
    pub fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = OsString>,
        timeout_secs: u64,
    ) -> Result<Self, EvaluationError> {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(EvaluationError::InvalidSpec(
                "sealed worker program must not be empty".into(),
            ));
        }
        if timeout_secs == 0 || timeout_secs > MAX_REQUEST_LIFETIME_SECS {
            return Err(EvaluationError::InvalidSpec(
                "sealed worker process timeout must be between 1 second and 24 hours".into(),
            ));
        }
        Ok(Self {
            program,
            args: args.into_iter().collect(),
            timeout_secs,
            max_receipt_bytes: DEFAULT_MAX_RECEIPT_BYTES,
        })
    }

    pub fn with_max_receipt_bytes(
        mut self,
        max_receipt_bytes: usize,
    ) -> Result<Self, EvaluationError> {
        if max_receipt_bytes == 0 || max_receipt_bytes > 64 * 1024 * 1024 {
            return Err(EvaluationError::InvalidSpec(
                "sealed receipt limit must be between 1 byte and 64 MiB".into(),
            ));
        }
        self.max_receipt_bytes = max_receipt_bytes;
        Ok(self)
    }
}

#[async_trait]
impl SealedEvaluationTransport for ProcessSealedTransport {
    async fn submit(
        &self,
        request: SealedEvaluationRequest,
    ) -> Result<SignedEvaluationReceipt, EvaluationError> {
        request.validate_at(now())?;
        let input = serde_json::to_vec(&request)
            .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
        if input.len() > MAX_REQUEST_WIRE_BYTES {
            return Err(EvaluationError::InvalidCandidate(
                "sealed request exceeds the 192 MiB process-transport limit".into(),
            ));
        }

        let mut child = tokio::process::Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                EvaluationError::Evaluator(format!(
                    "could not start sealed worker {}: {error}",
                    self.program.display()
                ))
            })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            EvaluationError::Evaluator("sealed worker stdin was unavailable".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            EvaluationError::Evaluator("sealed worker stdout was unavailable".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            EvaluationError::Evaluator("sealed worker stderr was unavailable".into())
        })?;
        let writer = tokio::spawn(async move {
            stdin.write_all(&input).await?;
            stdin.shutdown().await
        });
        let stdout_reader = tokio::spawn(read_bounded_async(stdout, self.max_receipt_bytes));
        let stderr_reader = tokio::spawn(read_bounded_async(stderr, MAX_WORKER_STDERR_BYTES));

        let status = match tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            child.wait(),
        )
        .await
        {
            Ok(result) => result.map_err(|error| EvaluationError::Evaluator(error.to_string()))?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(EvaluationError::Evaluator(format!(
                    "sealed worker process timed out after {}s",
                    self.timeout_secs
                )));
            }
        };
        writer
            .await
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))?
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))?;
        let stdout = stdout_reader
            .await
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))??;
        let stderr = stderr_reader
            .await
            .map_err(|error| EvaluationError::Evaluator(error.to_string()))??;
        if !status.success() {
            return Err(EvaluationError::Evaluator(format!(
                "sealed worker exited with {status}: {}",
                String::from_utf8_lossy(&stderr)
            )));
        }
        serde_json::from_slice(&stdout).map_err(|error| {
            EvaluationError::MalformedResult(format!(
                "sealed worker returned invalid receipt JSON: {error}"
            ))
        })
    }
}

/// Client proxy. It knows the public commitment and pinned worker key, but
/// never receives the hidden test vectors.
pub struct SealedEvaluatorClient {
    evaluator_id: String,
    evaluator_version: String,
    test_commitment_sha256: String,
    pinned_key: VerifyingKey,
    key_fingerprint_sha256: String,
    transport: Arc<dyn SealedEvaluationTransport>,
}

impl SealedEvaluatorClient {
    pub fn new(
        evaluator_id: impl Into<String>,
        evaluator_version: impl Into<String>,
        test_commitment_sha256: impl Into<String>,
        pinned_key: VerifyingKey,
        transport: Arc<dyn SealedEvaluationTransport>,
    ) -> Result<Self, EvaluationError> {
        let evaluator_id = evaluator_id.into();
        let evaluator_version = evaluator_version.into();
        let test_commitment_sha256 = test_commitment_sha256.into().to_ascii_lowercase();
        if evaluator_id.trim().is_empty()
            || evaluator_version.trim().is_empty()
            || evaluator_id.len() > MAX_ID_BYTES
            || evaluator_version.len() > MAX_ID_BYTES
        {
            return Err(EvaluationError::InvalidSpec(
                "sealed client evaluator identity is empty or too large".into(),
            ));
        }
        validate_sha256(&test_commitment_sha256, "hidden-test commitment")?;
        let key_fingerprint_sha256 = sha256(&pinned_key.to_bytes());
        Ok(Self {
            evaluator_id,
            evaluator_version,
            test_commitment_sha256,
            pinned_key,
            key_fingerprint_sha256,
            transport,
        })
    }

    fn request(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<SealedEvaluationRequest, EvaluationError> {
        if spec.timeout_secs > MAX_REQUEST_LIFETIME_SECS.saturating_sub(120) {
            return Err(EvaluationError::InvalidSpec(
                "sealed evaluation timeout is too large".into(),
            ));
        }
        let issued_at = now();
        let nonce = rand::rng().random::<[u8; 32]>();
        Ok(SealedEvaluationRequest {
            schema: SEALED_REQUEST_SCHEMA.into(),
            evaluator_id: self.evaluator_id.clone(),
            evaluator_version: self.evaluator_version.clone(),
            test_commitment_sha256: self.test_commitment_sha256.clone(),
            evaluation: spec.clone(),
            candidate: candidate.clone(),
            nonce: encode_hex(&nonce),
            issued_at,
            expires_at: issued_at
                .saturating_add(spec.timeout_secs)
                .saturating_add(120),
        })
    }
}

#[async_trait]
impl ObjectiveEvaluator for SealedEvaluatorClient {
    fn id(&self) -> &str {
        &self.evaluator_id
    }

    fn version(&self) -> &str {
        &self.evaluator_version
    }

    fn test_commitment_sha256(&self) -> Option<&str> {
        Some(&self.test_commitment_sha256)
    }

    fn attestation_key_sha256(&self) -> Option<&str> {
        Some(&self.key_fingerprint_sha256)
    }

    async fn evaluate(
        &self,
        spec: &EvaluationSpec,
        candidate: &CandidateArtifact,
    ) -> Result<ObjectiveEvaluation, EvaluationError> {
        let request = self.request(spec, candidate)?;
        let receipt = self.transport.submit(request.clone()).await?;
        verify_receipt(&request, &receipt, &self.pinned_key, now())?;
        embed_receipt(receipt)
    }
}

pub fn verify_receipt(
    request: &SealedEvaluationRequest,
    receipt: &SignedEvaluationReceipt,
    pinned_key: &VerifyingKey,
    verification_time: u64,
) -> Result<(), EvaluationError> {
    request.validate_at(verification_time)?;
    if receipt.payload.schema != SEALED_RECEIPT_SCHEMA {
        return Err(EvaluationError::MalformedResult(format!(
            "unsupported sealed receipt schema: {}",
            receipt.payload.schema
        )));
    }
    if receipt.payload.request_sha256 != request.sha256()? {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt request digest mismatch".into(),
        ));
    }
    if receipt.payload.evaluator_id != request.evaluator_id
        || receipt.payload.evaluator_version != request.evaluator_version
        || !receipt
            .payload
            .test_commitment_sha256
            .eq_ignore_ascii_case(&request.test_commitment_sha256)
    {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt evaluator or test commitment mismatch".into(),
        ));
    }
    if receipt.payload.worker_id.trim().is_empty() || receipt.payload.worker_id.len() > MAX_ID_BYTES
    {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt worker identity is invalid".into(),
        ));
    }
    if receipt.payload.issued_at < request.issued_at
        || receipt.payload.issued_at > request.expires_at.saturating_add(MAX_CLOCK_SKEW_SECS)
        || receipt.payload.issued_at > verification_time.saturating_add(MAX_CLOCK_SKEW_SECS)
    {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt issuance time is outside the request window".into(),
        ));
    }

    let encoded_key = decode_hex::<32>(&receipt.verifying_key_hex, "worker verifying key")?;
    let receipt_key = VerifyingKey::from_bytes(&encoded_key)
        .map_err(|error| EvaluationError::MalformedResult(error.to_string()))?;
    if receipt_key != *pinned_key {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt was not signed by the pinned worker key".into(),
        ));
    }
    let key_fingerprint = sha256(&receipt_key.to_bytes());
    if receipt.payload.worker_key_sha256 != key_fingerprint {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt worker-key fingerprint mismatch".into(),
        ));
    }

    receipt.evaluation.validate_against(&request.evaluation)?;
    if receipt.evaluation.evaluator_id != request.evaluator_id
        || receipt.evaluation.evaluator_version != request.evaluator_version
        || receipt.evaluation.candidate_id != request.candidate.id
        || receipt.evaluation.candidate_sha256 != request.candidate.sha256()
    {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt evaluation provenance mismatch".into(),
        ));
    }
    if receipt.payload.evaluation_sha256 != evaluation_sha256(&receipt.evaluation)? {
        return Err(EvaluationError::MalformedResult(
            "sealed receipt evaluation digest mismatch".into(),
        ));
    }

    let signature_bytes = decode_hex::<64>(&receipt.signature_hex, "receipt signature")?;
    let signature = Signature::from_bytes(&signature_bytes);
    let payload_bytes = serde_json::to_vec(&receipt.payload)
        .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
    receipt_key
        .verify(&payload_bytes, &signature)
        .map_err(|_| EvaluationError::MalformedResult("sealed receipt signature is invalid".into()))
}

/// Verify the self-contained attestation fields embedded by a sealed client in
/// a public objective result.
pub fn verify_embedded_attestation(
    evaluation: &ObjectiveEvaluation,
) -> Result<(), EvaluationError> {
    let value = |key: &str| {
        evaluation.environment.get(key).cloned().ok_or_else(|| {
            EvaluationError::MalformedResult(format!(
                "objective evaluation is missing attestation field {key}"
            ))
        })
    };
    if value("sealed_attestation_schema")? != SEALED_RECEIPT_SCHEMA {
        return Err(EvaluationError::MalformedResult(
            "embedded attestation schema mismatch".into(),
        ));
    }
    let mut original = evaluation.clone();
    for key in ATTESTATION_ENV_KEYS {
        original.environment.remove(key);
    }
    let payload = SealedReceiptPayload {
        schema: SEALED_RECEIPT_SCHEMA.into(),
        request_sha256: value("sealed_request_sha256")?,
        worker_id: value("sealed_worker_id")?,
        worker_key_sha256: value("sealed_worker_key_sha256")?,
        evaluator_id: evaluation.evaluator_id.clone(),
        evaluator_version: evaluation.evaluator_version.clone(),
        test_commitment_sha256: value("sealed_test_commitment_sha256")?,
        evaluation_sha256: value("sealed_evaluation_sha256")?,
        issued_at: value("sealed_receipt_issued_at")?.parse().map_err(|_| {
            EvaluationError::MalformedResult("invalid embedded receipt timestamp".into())
        })?,
    };
    if payload.evaluation_sha256 != evaluation_sha256(&original)? {
        return Err(EvaluationError::MalformedResult(
            "embedded attestation evaluation digest mismatch".into(),
        ));
    }
    let key_bytes = decode_hex::<32>(&value("sealed_verifying_key_hex")?, "worker verifying key")?;
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|error| EvaluationError::MalformedResult(error.to_string()))?;
    if payload.worker_key_sha256 != sha256(&key.to_bytes()) {
        return Err(EvaluationError::MalformedResult(
            "embedded worker-key fingerprint mismatch".into(),
        ));
    }
    let signature_bytes =
        decode_hex::<64>(&value("sealed_receipt_signature")?, "receipt signature")?;
    let signature = Signature::from_bytes(&signature_bytes);
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
    key.verify(&payload_bytes, &signature)
        .map_err(|_| EvaluationError::MalformedResult("embedded signature is invalid".into()))?;

    let receipt = SignedEvaluationReceipt {
        payload,
        verifying_key_hex: encode_hex(&key.to_bytes()),
        signature_hex: encode_hex(&signature.to_bytes()),
        evaluation: original,
    };
    if value("sealed_receipt_sha256")? != receipt.sha256()? {
        return Err(EvaluationError::MalformedResult(
            "embedded receipt digest mismatch".into(),
        ));
    }
    Ok(())
}

fn embed_receipt(receipt: SignedEvaluationReceipt) -> Result<ObjectiveEvaluation, EvaluationError> {
    let receipt_sha256 = receipt.sha256()?;
    let mut evaluation = receipt.evaluation.clone();
    let values = [
        ("sealed_attestation_schema", receipt.payload.schema.clone()),
        ("sealed_worker_id", receipt.payload.worker_id.clone()),
        (
            "sealed_worker_key_sha256",
            receipt.payload.worker_key_sha256.clone(),
        ),
        (
            "sealed_verifying_key_hex",
            receipt.verifying_key_hex.clone(),
        ),
        (
            "sealed_request_sha256",
            receipt.payload.request_sha256.clone(),
        ),
        (
            "sealed_evaluation_sha256",
            receipt.payload.evaluation_sha256.clone(),
        ),
        (
            "sealed_receipt_issued_at",
            receipt.payload.issued_at.to_string(),
        ),
        ("sealed_receipt_signature", receipt.signature_hex.clone()),
        ("sealed_receipt_sha256", receipt_sha256),
        (
            "sealed_test_commitment_sha256",
            receipt.payload.test_commitment_sha256.clone(),
        ),
    ];
    for (key, value) in values {
        if evaluation.environment.insert(key.into(), value).is_some() {
            return Err(EvaluationError::MalformedResult(
                "underlying evaluator collided with sealed attestation fields".into(),
            ));
        }
    }
    verify_embedded_attestation(&evaluation)?;
    Ok(evaluation)
}

fn evaluation_sha256(evaluation: &ObjectiveEvaluation) -> Result<String, EvaluationError> {
    let bytes = serde_json::to_vec(evaluation)
        .map_err(|error| EvaluationError::Serialization(error.to_string()))?;
    Ok(sha256(&bytes))
}

async fn read_bounded_async<R: AsyncRead + Unpin>(
    reader: R,
    limit: usize,
) -> Result<Vec<u8>, EvaluationError> {
    let mut output = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut output)
        .await
        .map_err(|error| EvaluationError::Evaluator(error.to_string()))?;
    if output.len() > limit {
        return Err(EvaluationError::Evaluator(format!(
            "sealed worker output exceeded {limit} bytes"
        )));
    }
    Ok(output)
}

fn validate_sha256(value: &str, label: &str) -> Result<(), EvaluationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EvaluationError::InvalidSpec(format!(
            "{label} is not a SHA-256 digest"
        )));
    }
    Ok(())
}

fn decode_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], EvaluationError> {
    if value.len() != N * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EvaluationError::MalformedResult(format!(
            "{label} is not valid hexadecimal"
        )));
    }
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|error| EvaluationError::MalformedResult(error.to_string()))?;
    }
    Ok(output)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
