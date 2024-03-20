"""
Iterative Problem Solver using OpenAI and Local Summarization

This script provides an interactive way to solve theoretical problems iteratively by querying the OpenAI API and summarizing previous iterations for context using a local BART model for text summarization. The script is designed with an OOP approach, enhancing modularity and readability.

Classes:
- Summarizer: Handles text summarization using the BART model.
- Conversation: Manages conversation history and context summarization.
- OpenAIQueryHandler: Facilitates querying the OpenAI API.
- ProblemSolver: Orchestrates the problem-solving process based on user input.
- main: Entry point for the script.
"""

import os
from transformers import BartTokenizer, BartForConditionalGeneration
from openai import OpenAI

class Summarizer:
    """
    Summarizes text using the BART model.
    
    Methods:
    - summarize(text): Returns a summary of the given text.
    """
    def __init__(self):
        self.tokenizer = BartTokenizer.from_pretrained('facebook/bart-large-cnn')
        self.model = BartForConditionalGeneration.from_pretrained('facebook/bart-large-cnn')

    def summarize(self, text):
        inputs = self.tokenizer([text], max_length=1024, return_tensors='pt', truncation=True)
        summary_ids = self.model.generate(inputs['input_ids'], num_beams=4, max_length=150, early_stopping=True)
        summary = self.tokenizer.decode(summary_ids[0], skip_special_tokens=True)
        return summary

class Conversation:
    """
    Manages conversation history and generates context for prompts.
    
    Methods:
    - add_entry(entry): Adds an entry to the conversation history.
    - summarize_context(): Summarizes the conversation history to provide context.
    """
    def __init__(self, summarizer):
        self.history = []
        self.summarizer = summarizer

    def add_entry(self, entry):
        self.history.append(entry)

    def summarize_context(self):
        full_conversation = " ".join(self.history)
        return self.summarizer.summarize(full_conversation) if self.history else ""

class OpenAIQueryHandler:
    """
    Handles queries to the OpenAI API.
    
    Methods:
    - query(prompt): Returns a response to the given prompt from the OpenAI API.
    """
    def __init__(self, api_key, organization):
        self.client = OpenAI(api_key=api_key, organization=organization)

    def query(self, prompt):
        response = self.client.create_completion(
            prompt=prompt,
            model="text-davinci-003",
            temperature=0.7,
            max_tokens=150,
            n=1,
            stop=None
        )
        return response.choices[0].text.strip()

class ProblemSolver:
    """
    Orchestrates the problem-solving process using conversation management and OpenAI queries.
    
    Methods:
    - solve(logfile, first_line, num_iterations): Solves the problem iteratively based on the given parameters.
    """
    def __init__(self, api_key, organization):
        self.summarizer = Summarizer()
        self.conversation = Conversation(self.summarizer)
        self.query_handler = OpenAIQueryHandler(api_key, organization)

    def append_to_file(self, filepath, content):
        with open(filepath, "a") as file:
            file.write(content + "\n")

    def generate_prompt(self, first_line, context):
        return f"Given the context: {context}, continue exploring the theoretical problem starting with: '{first_line}'. Aim to deepen the understanding and propose next steps for consideration."

    def solve(self, logfile, first_line, num_iterations):
        if not os.path.exists(logfile):
            print(f"{logfile} does not exist. Creating new file.")
            os.makedirs(os.path.dirname(logfile), exist_ok=True)

        for _ in range(num_iterations):
            context = self.conversation.summarize_context()
            prompt = self.generate_prompt(first_line, context)
            content = self.query_handler.query(prompt)
            self.append_to_file(logfile, content)
            self.conversation.add_entry(content)
            first_line = "Considering the above, what would be the next logical step?"

def help():
    """
    Displays help information on how to use the script.
    """
    print("""
Usage: iterative_problem_solver_oop.py

Interactive script to solve theoretical problems iteratively with the help of the OpenAI API and local summarization.

User Inputs:
- OpenAI API key
- OpenAI Organization ID
- Logfile path
- Number of iterations
- Starting line for the conversation

The script will guide you through each step.
    """)

def main():
    help()  # Display help information at the start.
    api_key = input("Enter your OpenAI API key: ")
    organization = input("Enter your OpenAI organization ID: ")
    logfile = input("Enter the path to your logfile: ")
    num_iterations = int(input("How many iterations would you like to run? "))
    first_line = input("Enter the first line to start the conversation: ")

    problem_solver = ProblemSolver(api_key, organization)
    problem_solver.solve(logfile, first_line, num_iterations)

if __name__ == "__main__":
    main()
