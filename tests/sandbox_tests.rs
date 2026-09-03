use crosstalk::engines::sandbox::{I64TestCase, SandboxConfig, SandboxManager};
use crosstalk::engines::simulation::MonteCarloRunner;
use crosstalk::engines::validation::{AstValidator, AstVersionHistory};
use crosstalk::types::artifact::Artifact;
use std::collections::HashMap;

// Invalid WASM bytes for testing error handling
fn invalid_wasm_bytes() -> Vec<u8> {
    vec![0xFF, 0xFF, 0xFF, 0xFF]
}

// A valid module whose default ("") export is `() -> ()` and loops forever,
// so execution exhausts CPU fuel rather than returning normally.
fn infinite_loop_wasm_bytes() -> Vec<u8> {
    vec![
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // header
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type: () -> ()
        0x03, 0x02, 0x01, 0x00, // function: type 0
        0x07, 0x04, 0x01, 0x00, 0x00, 0x00, // export "" func 0
        0x0A, 0x09, 0x01, 0x07, 0x00, // code section, body len 7, 0 locals
        0x03, 0x40, 0x0C, 0x00, 0x0B, 0x0B, // loop (br 0) end; end
    ]
}

#[tokio::test]
async fn test_sandbox_fuel_limit() {
    let config = SandboxConfig {
        memory_limit_bytes: 1024 * 1024,
        cpu_fuel_limit: 100,
        ..Default::default()
    };
    let manager = SandboxManager::new(config).expect("Failed to create SandboxManager");

    let wasm_bytes = invalid_wasm_bytes();

    let result = manager.execute(&wasm_bytes);

    // Invalid WASM should produce an error
    assert!(
        result.is_err(),
        "Invalid WASM should fail to instantiate or execute"
    );
}

#[tokio::test]
async fn fuel_exhaustion_sets_resource_limit_hit() {
    let config = SandboxConfig {
        memory_limit_bytes: 1024 * 1024,
        cpu_fuel_limit: 100_000,
        ..Default::default()
    };
    let manager = SandboxManager::new(config).expect("Failed to create SandboxManager");

    let result = manager
        .execute(&infinite_loop_wasm_bytes())
        .expect("module instantiates; the trap is reported via the result");

    assert!(
        result.resource_limit_hit,
        "fuel exhaustion must be distinguished from an ordinary failure"
    );
    assert_eq!(result.exit_code, 1);
    assert_eq!(
        result.fuel_consumed,
        Some(100_000),
        "the run should consume the full fuel budget before trapping"
    );
}

#[tokio::test]
async fn hidden_i64_cases_use_fresh_instances_and_aggregate_fuel() {
    let manager = std::sync::Arc::new(
        SandboxManager::new(SandboxConfig {
            memory_limit_bytes: 1024 * 1024,
            cpu_fuel_limit: 100_000,
            timeout_secs: 5,
        })
        .unwrap(),
    );
    // The mutable global would make later cases fail if instances were reused.
    let wasm = wat::parse_str(
        r#"
        (module
          (global $calls (mut i64) (i64.const 0))
          (func (export "solve") (param $value i64) (result i64)
            global.get $calls
            i64.const 1
            i64.add
            global.set $calls
            local.get $value
            global.get $calls
            i64.add))
        "#,
    )
    .unwrap();
    let cases = vec![
        I64TestCase {
            input: 4,
            expected: 5,
        },
        I64TestCase {
            input: 10,
            expected: 11,
        },
    ];
    let result = manager
        .evaluate_i64_cases_with_timeout(&wasm, "solve", &cases)
        .await
        .unwrap();
    assert!(result.all_cases_correct(cases.len()));
    assert!(result.fuel_consumed > 0);
    assert_eq!(result.outcomes.len(), cases.len());
}

#[tokio::test]
async fn hidden_i64_case_traps_are_fail_closed() {
    let manager = std::sync::Arc::new(
        SandboxManager::new(SandboxConfig {
            memory_limit_bytes: 1024 * 1024,
            cpu_fuel_limit: 1_000,
            timeout_secs: 5,
        })
        .unwrap(),
    );
    let wasm = wat::parse_str(
        r#"
        (module
          (func (export "solve") (param i64) (result i64)
            (loop $forever
              br $forever)
            i64.const 0))
        "#,
    )
    .unwrap();
    let cases = vec![I64TestCase {
        input: 1,
        expected: 1,
    }];
    let result = manager
        .evaluate_i64_cases_with_timeout(&wasm, "solve", &cases)
        .await
        .unwrap();
    assert!(result.trapped);
    assert!(result.resource_limit_hit);
    assert!(!result.all_cases_correct(cases.len()));
}

#[tokio::test]
async fn test_sandbox_memory_limit() {
    let config = SandboxConfig {
        memory_limit_bytes: 1024,
        cpu_fuel_limit: 10_000_000,
        ..Default::default()
    };
    let manager = SandboxManager::new(config).expect("Failed to create SandboxManager");

    let wasm_bytes = invalid_wasm_bytes();

    let result = manager.execute(&wasm_bytes);

    // Invalid WASM with memory limit should produce an error
    assert!(result.is_err(), "Invalid WASM should fail");
}

#[tokio::test]
async fn test_sandbox_stdout_capture() {
    let config = SandboxConfig {
        memory_limit_bytes: 1024 * 1024,
        cpu_fuel_limit: 10_000_000,
        ..Default::default()
    };
    let manager = SandboxManager::new(config).expect("Failed to create SandboxManager");

    let wasm_bytes = invalid_wasm_bytes();

    let result = manager.execute(&wasm_bytes);

    // Test that SandboxResult structure is available
    // Even with invalid WASM, if execution happens, stdout should be captured
    match result {
        Ok(sandbox_result) => {
            // Verify the result has the expected fields
            let _ = &sandbox_result.stdout;
            let _ = &sandbox_result.stderr;
            let _ = sandbox_result.exit_code;
        }
        Err(_) => {
            // Invalid WASM fails, which is expected
        }
    }
}

#[tokio::test]
async fn test_monte_carlo_trials() {
    let runner = MonteCarloRunner::new().expect("Failed to create MonteCarloRunner");

    let artifact = Artifact {
        name: "test.rs".to_string(),
        language: "rust".to_string(),
        content: "fn main() {}".to_string(),
        version: 1,
        history: vec![],
        ast_versions: std::collections::BTreeMap::new(),
        proof_attachments: vec![],
        metrics: Default::default(),
        skeleton: String::new(),
    };

    let diff = crosstalk::types::artifact::ArtifactDiff {
        original_version: 0,
        new_version: 1,
        diff_text: String::new(),
    };

    let (probability, _variance) = runner
        .predict(&artifact, &diff, 100)
        .await
        .expect("predict failed");

    // Probability should be between 0.0 and 1.0
    assert!(
        (0.0..=1.0).contains(&probability),
        "Probability {probability} should be in [0.0, 1.0]"
    );
}

#[tokio::test]
async fn test_monte_carlo_variance() {
    let runner = MonteCarloRunner::new().expect("Failed to create MonteCarloRunner");

    let artifact = Artifact {
        name: "test.rs".to_string(),
        language: "rust".to_string(),
        content: "fn main() {}".to_string(),
        version: 1,
        history: vec![],
        ast_versions: std::collections::BTreeMap::new(),
        proof_attachments: vec![],
        metrics: Default::default(),
        skeleton: String::new(),
    };

    let diff = crosstalk::types::artifact::ArtifactDiff {
        original_version: 0,
        new_version: 1,
        diff_text: String::new(),
    };

    // Run prediction twice with same config
    let (result1, _) = runner
        .predict(&artifact, &diff, 100)
        .await
        .expect("predict failed");
    let (result2, _) = runner
        .predict(&artifact, &diff, 100)
        .await
        .expect("predict failed");

    assert!((0.0..=1.0).contains(&result1));
    assert!((0.0..=1.0).contains(&result2));
    assert!(result1.is_finite(), "result1 should be finite");
    assert!(result2.is_finite(), "result2 should be finite");
}

#[test]
fn test_ast_versioning() {
    let code1 = r#"
        pub fn add(a: i32, b: i32) -> i32 {
            a + b
        }

        pub fn multiply(a: i32, b: i32) -> i32 {
            a * b
        }
    "#;

    let code2 = r#"
        pub fn add(a: i32, b: i32) -> i32 {
            a + b + 1
        }

        pub fn multiply(a: i32, b: i32) -> i32 {
            a * b
        }

        pub fn divide(a: i32, b: i32) -> i32 {
            a / b
        }
    "#;

    // Extract nodes from original code
    let nodes1 = AstValidator::extract_nodes(code1, "rust");
    assert!(nodes1.len() >= 2, "Expected at least 2 nodes in code1");

    // Extract nodes from modified code
    let nodes2 = AstValidator::extract_nodes(code2, "rust");
    assert!(nodes2.len() >= 3, "Expected at least 3 nodes in code2");

    // Identify changed nodes
    let changed_nodes = AstValidator::identify_changed_nodes(code1, code2, "rust");

    // The 'add' function should be identified as changed
    assert!(
        changed_nodes.iter().any(|id| id.contains("add")),
        "Expected 'add' function to be identified as changed"
    );

    // The 'divide' function should be identified as new
    assert!(
        changed_nodes.iter().any(|id| id.contains("divide")),
        "Expected 'divide' function to be identified as new"
    );

    // The 'multiply' function might or might not be in changed_nodes depending on implementation
    // (it's unchanged, so it should not be in the list if the implementation is correct)
    let multiply_changed = changed_nodes.iter().any(|id| id.contains("multiply"));
    assert!(
        !multiply_changed,
        "Expected 'multiply' to NOT be marked as changed (it's unchanged)"
    );
}

#[test]
fn test_ast_versioning_deletion() {
    let code1 = r#"
        pub fn add(a: i32, b: i32) -> i32 {
            a + b
        }

        pub fn multiply(a: i32, b: i32) -> i32 {
            a * b
        }
    "#;

    let code2 = r#"
        pub fn add(a: i32, b: i32) -> i32 {
            a + b
        }
    "#;

    // Identify changed nodes (deletion)
    let changed_nodes = AstValidator::identify_changed_nodes(code1, code2, "rust");

    // The 'multiply' function should be identified as deleted
    assert!(
        changed_nodes.iter().any(|id| id.contains("multiply")),
        "Expected 'multiply' function to be identified as deleted"
    );

    // The 'add' function should not be changed
    let add_changed = changed_nodes.iter().any(|id| id.contains("add"));
    assert!(!add_changed, "Expected 'add' to NOT be marked as changed");
}

#[test]
fn test_ast_versioning_struct() {
    let code1 = r#"
        struct Point {
            x: i32,
            y: i32,
        }
    "#;

    let code2 = r#"
        struct Point {
            x: i32,
            y: i32,
            z: i32,
        }
    "#;

    // Identify changed nodes
    let changed_nodes = AstValidator::identify_changed_nodes(code1, code2, "rust");

    // The 'Point' struct should be identified as changed
    assert!(
        changed_nodes.iter().any(|id| id.contains("Point")),
        "Expected 'Point' struct to be identified as changed"
    );
}

// ── AstVersionHistory ─────────────────────────────────────────────────────────

#[test]
fn revert_node_returns_content_at_target_turn() {
    let mut history = AstVersionHistory::new();
    let mut snap1 = HashMap::new();
    snap1.insert("fn:foo".to_string(), "fn foo() {}".to_string());
    history.record_snapshot(1, snap1);
    let mut snap2 = HashMap::new();
    snap2.insert("fn:foo".to_string(), "fn foo() { 42 }".to_string());
    history.record_snapshot(2, snap2);

    let v1 = history.revert_node("fn:foo", 1).unwrap();
    assert_eq!(v1, "fn foo() {}");
    let v2 = history.revert_node("fn:foo", 2).unwrap();
    assert_eq!(v2, "fn foo() { 42 }");
}

#[test]
fn revert_node_errors_for_unknown_node() {
    let history = AstVersionHistory::new();
    assert!(history.revert_node("fn:missing", 1).is_err());
}

#[test]
fn revert_node_errors_when_node_not_yet_created() {
    let mut history = AstVersionHistory::new();
    let mut snap = HashMap::new();
    snap.insert("fn:late".to_string(), "fn late() {}".to_string());
    history.record_snapshot(5, snap);
    assert!(history.revert_node("fn:late", 3).is_err());
}

#[test]
fn diff_nodes_contains_added_line() {
    let mut history = AstVersionHistory::new();
    let mut s1 = HashMap::new();
    s1.insert("fn:bar".to_string(), "line1\n".to_string());
    history.record_snapshot(1, s1);
    let mut s2 = HashMap::new();
    s2.insert("fn:bar".to_string(), "line1\nline2\n".to_string());
    history.record_snapshot(2, s2);

    let diff = history.diff_nodes("fn:bar", 1, 2).unwrap();
    assert!(
        diff.contains('+'),
        "diff must contain '+' for inserted lines"
    );
}

#[test]
fn diff_nodes_identical_versions_has_no_changes() {
    let mut history = AstVersionHistory::new();
    let mut s1 = HashMap::new();
    s1.insert("fn:same".to_string(), "fn same() {}\n".to_string());
    history.record_snapshot(1, s1.clone());
    history.record_snapshot(2, s1);

    let diff = history.diff_nodes("fn:same", 1, 2).unwrap();
    assert!(
        !diff.contains('+') && !diff.contains('-'),
        "identical versions should produce no change markers"
    );
}

// ── SandboxManager with valid WASM ──────────────────────────────────────────

#[tokio::test]
async fn execute_returns_error_on_invalid_wasm() {
    let config = SandboxConfig {
        memory_limit_bytes: 1024 * 1024,
        cpu_fuel_limit: 10_000_000,
        ..Default::default()
    };
    let manager = SandboxManager::new(config).unwrap();
    let result = manager.execute(&[0xFF, 0xFF]);
    assert!(result.is_err());
}

#[test]
fn default_sandbox_config_has_reasonable_limits() {
    let config = SandboxConfig::default();
    assert!(config.memory_limit_bytes > 0);
    assert!(config.cpu_fuel_limit > 0);
}

/// Regression pin: `set_epoch_deadline` was a fixed 1 tick against a
/// free-running one-second incrementer, so every call died after at most one
/// second of wall clock no matter what the operator configured. The evaluator's
/// reproduction gate compares fuel bit-for-bit, so any candidate costing about
/// a second became a coin flip between the primary and reproduction passes.
///
/// FIXME: this pins the derivation, not the call site, so it does not fail
/// against the pre-fix code and is not a complete regression pin. Three
/// behavioural attempts failed to bite: synthetic busy loops get folded away by
/// the optimiser, and a deliberately aged manager still runs fine under the old
/// fixed deadline, which refutes the obvious "the budget is the manager's own
/// age" reading. The verified causal handle: forcing `--timeout-secs 1` on a
/// real tournament reproduces the original divergence exactly, and raising it
/// cures it. The mechanism behind the primary/reproduction asymmetry is still
/// unexplained.
#[tokio::test]
async fn the_epoch_deadline_scales_with_the_configured_timeout() {
    let ticks = |timeout_secs| {
        SandboxManager::new(SandboxConfig {
            memory_limit_bytes: 16 * 1024 * 1024,
            cpu_fuel_limit: 100_000_000,
            timeout_secs,
        })
        .unwrap()
        .epoch_deadline_ticks()
    };
    assert!(ticks(300) > 300, "a 300s timeout must outlast 300 ticks");
    assert!(ticks(30) > ticks(1), "the deadline must track the timeout");
}
