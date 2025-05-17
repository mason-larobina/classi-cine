# Contribution Conventions for classi-cine

This document outlines the conventions and workflow for contributing to the
`classi-cine` project. Adhering to these guidelines helps maintain code quality,
facilitates collaboration, and ensures a smooth development process.

## Code Style

- **Adhere to Existing Style:** When making changes, please observe and follow
  the existing code style, naming conventions, and project structure present in
  the codebase. Consistency is key to maintaining a readable and maintainable
  codebase.
- **Error Handling:** Continue to use the `Result` type and the `thiserror`
  crate for error handling, consistent with the current implementation. Avoid
  using `unwrap()` or `expect()` in production code.
- **Logging:** Utilize the `log` crate for logging information, warnings, and
  errors, following the existing patterns for different log levels.

## Contribution Workflow

We strongly encourage contributions to be submitted as **single, logical
features or bug fixes**. This approach significantly simplifies the review
process and allows for focused discussion and guidance.

1. **Understand the Goal:** Before starting work, ensure you have a clear
   understanding of the feature you want to implement or the bug you want to
   fix. If you're unsure, don't hesitate to ask clarifying questions.
1. **Describe Your Plan:** Before writing a significant amount of code, it's
   often helpful to briefly describe your planned approach. This allows for
   early feedback and ensures alignment with the project's direction.
1. **Implement a Single Logical Feature:** Focus on implementing one distinct
   feature or fixing one specific bug in your contribution. Avoid bundling
   unrelated changes together.
1. **Test Your Changes:** Ensure your changes are well-tested. If you're adding
   a new feature, include tests that cover its functionality. If you're fixing a
   bug, add a test that reproduces the bug and verifies the fix.
1. **Submit Your Changes:** Submit your changes for review.
1. **Explain Your Contribution:** In your submission, clearly explain what your
   changes do, why they were made, and how they fit into the overall project.
1. **Be Open to Feedback:** Be prepared to receive feedback on your code. The
   review process is an opportunity for collaboration and improvement.

**Emphasis on Single Logical Features and Communication:**

Submitting a single logical feature at a time makes it much easier for reviewers
to understand the scope of your changes and provide targeted feedback. If you
have multiple ideas or bug fixes, please address them in separate contributions.

We value clear communication. If you have questions about the code, the
project's direction, or your planned approach, please ask! We're here to help
and guide you.

We appreciate your contributions and look forward to collaborating with you!
