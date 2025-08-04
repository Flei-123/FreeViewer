# Contributing to FreeViewer

Thank you for your interest in contributing to FreeViewer! 🎉

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/yourusername/FreeViewer.git
   cd FreeViewer
   ```
3. **Install Rust** (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
4. **Build the project**:
   ```bash
   cargo build
   ```
5. **Run tests**:
   ```bash
   cargo test
   ```

## Development Environment

### Prerequisites
- Rust 1.70+
- Git
- Platform-specific dependencies:
  - **Windows**: Visual Studio Build Tools
  - **Linux**: X11 development libraries (`libx11-dev`, `libxrandr-dev`)
  - **macOS**: Xcode command line tools

### Recommended Setup
- **VS Code** with the rust-analyzer extension
- **Git** for version control
- **GitHub CLI** for easier PR management

## Code Style

We follow standard Rust conventions:

- **Formatting**: Use `cargo fmt` before committing
- **Linting**: Fix warnings with `cargo clippy`
- **Documentation**: Document public APIs with `///` comments
- **Testing**: Write tests for new functionality

### Example:
```rust
/// Connects to a remote computer using the specified ID and password.
/// 
/// # Arguments
/// * `partner_id` - The 9-digit ID of the remote computer
/// * `password` - The access password
/// 
/// # Returns
/// * `Ok(())` if connection successful
/// * `Err(ClientError)` if connection fails
/// 
/// # Example
/// ```rust
/// let client = FreeViewerClient::new();
/// client.connect("123 456 789".to_string(), "secret".to_string()).await?;
/// ```
pub async fn connect(&self, partner_id: String, password: String) -> Result<(), ClientError> {
    // Implementation here
}
```

## Commit Messages

Use clear, descriptive commit messages:

- **feat**: New feature
- **fix**: Bug fix
- **docs**: Documentation changes
- **style**: Code style changes (formatting, etc.)
- **refactor**: Code refactoring
- **test**: Adding or updating tests
- **chore**: Maintenance tasks

Examples:
```
feat: add file transfer progress indicator
fix: resolve screen capture memory leak on Windows
docs: update installation instructions for Linux
```

## Pull Request Process

1. **Create a feature branch**:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** and commit them:
   ```bash
   git add .
   git commit -m "feat: add your feature description"
   ```

3. **Push to your fork**:
   ```bash
   git push origin feature/your-feature-name
   ```

4. **Create a Pull Request** on GitHub with:
   - Clear title and description
   - Screenshots/GIFs for UI changes
   - Test results
   - Breaking changes (if any)

5. **Address review feedback** if needed

## Testing

### Running Tests
```bash
# All tests
cargo test

# Specific module
cargo test protocol::tests

# With output
cargo test -- --nocapture

# Integration tests
cargo test --test integration
```

### Writing Tests
- **Unit tests**: Place in the same file as the code being tested
- **Integration tests**: Place in the `tests/` directory
- **UI tests**: Use the GUI testing framework

Example:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_success() {
        let client = FreeViewerClient::new();
        // Test implementation
    }
}
```

## Issue Guidelines

### Reporting Bugs
When reporting bugs, include:
- **OS and version**
- **FreeViewer version**
- **Steps to reproduce**
- **Expected vs actual behavior**
- **Screenshots/logs** if applicable

### Feature Requests
For feature requests, provide:
- **Use case description**
- **Proposed solution**
- **Alternative solutions considered**
- **Additional context**

## Security

For security vulnerabilities:
1. **DO NOT** create public issues
2. **Email** security@freeviewer.org
3. **Include** detailed description and steps to reproduce
4. **Wait** for acknowledgment before disclosure

## Architecture Overview

```
FreeViewer Architecture
├── src/
│   ├── main.rs           # Main application entry
│   ├── ui/               # GUI components (egui)
│   ├── client/           # Client-side connection logic
│   ├── host/             # Host-side services
│   ├── protocol/         # Network protocol implementation
│   ├── security/         # Encryption and authentication
│   └── capture/          # Screen and audio capture
├── tests/                # Integration tests
└── docs/                 # Documentation
```

## Areas Needing Contribution

- 🖥️ **Cross-platform compatibility** (Linux, macOS testing)
- 🔒 **Security auditing** and improvements
- 🌐 **Network optimization** and NAT traversal
- 📱 **Mobile apps** (Android, iOS)
- 🎨 **UI/UX improvements**
- 📚 **Documentation** and tutorials
- 🧪 **Testing** and quality assurance
- 🌍 **Internationalization** (i18n)

## Community

- **Discord**: [Join our Discord](https://discord.gg/freeviewer)
- **GitHub Discussions**: For questions and ideas
- **GitHub Issues**: For bugs and feature requests
- **Twitter**: [@FreeViewerApp](https://twitter.com/freeviewerapp)

## Recognition

Contributors will be:
- Listed in the project README
- Mentioned in release notes
- Invited to the contributors team (for regular contributors)

## Code of Conduct

We follow the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct):
- Be welcoming and inclusive
- Be respectful and constructive
- Focus on the community's best interests

---

**Questions?** Feel free to ask in GitHub Discussions or Discord!

Happy coding! 🦀✨
