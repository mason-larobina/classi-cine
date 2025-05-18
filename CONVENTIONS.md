# AI Code Analyst Interaction Conventions for classi-cine

This document outlines the conventions for the AI code analyst's interaction and
code generation process when contributing to the `classi-cine` project. Adhering
to these guidelines helps ensure efficient collaboration, clear communication,
and high-quality code output.

## Core Principles for AI Interaction

- **Minimize Rework:** Prioritize understanding the request and identifying
  potential issues *before* generating significant code.
- **Focus on Single, Logical Changes:** Break down complex tasks into smaller,
  manageable steps.
- **Proactive Problem Identification:** Highlight potential issues, ambiguities,
  or alternative approaches early in the process.
- **Clear Communication:** Explain proposed plans, generated code, and suggested
  next steps concisely.

## Workflow for Handling Requests

When a coding request is received, the AI should follow this general workflow:

1. **Confirm Understanding:** If the request is complex, ambiguous, or involves
   multiple steps, rephrase the request or ask clarifying questions to ensure a
   shared understanding of the goal.
1. **Propose a Plan (for non-trivial changes):** For requests that involve more
   than a simple, isolated code change, propose a brief, step-by-step plan
   before generating code. This allows for early feedback and alignment.
1. **Identify Potential Issues:** Analyze the request in the context of the
   existing codebase and conventions. Proactively point out any potential
   conflicts, design concerns, or areas of ambiguity *before* implementation, or
   highlight them clearly when presenting the changes.
1. **Implement the Change:** Generate the code necessary to fulfill the request,
   adhering to the code style and quality conventions below.
1. **Consider Testing:** If the change adds new functionality or fixes a bug,
   consider how it could be tested. Suggest adding tests or include test code if
   appropriate and feasible within the scope of the request.
1. **Present the Changes:** Provide a clear and brief summary of the generated
   code changes. Explain *what* was changed and *why*, linking it back to the
   original request and the agreed-upon plan (if any).
1. **Suggest Follow-up Work (Optional):** After completing a specific request,
   suggest logical next steps or related tasks that could be addressed in future
   interactions.

## Code Style and Quality Conventions

When generating code, the AI must adhere to the following:

- **Adhere to Existing Style:** Observe and follow the existing code style,
  naming conventions, and project structure.
- **Error Handling:** Continue to use the `Result` type and the `thiserror`
  crate for error handling. **Avoid** using `unwrap()` or `expect()` in
  production code paths.
- **Logging:** Utilize the `log` crate for logging information, warnings, and
  errors, following existing patterns.
- **Readability:** Generate code that is clean, well-structured, and easy for a
  human to read and understand.

## Work Unit Size

Contributions should be limited to **single, logical features or bug fixes**. If
a request implies a larger change, the AI should suggest breaking it down and
focus on completing the first logical step. This facilitates easier review and
reduces the risk of introducing unintended side effects.

By following these conventions, the AI can provide more effective and efficient
assistance in developing the `classi-cine` project.
