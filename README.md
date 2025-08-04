# FreeViewer 🖥️

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-windows%20%7C%20linux%20%7C%20macos-lightgrey)](https://github.com/yourusername/freeviewer)

**Open-source remote desktop software - A free TeamViewer alternative built in Rust**

FreeViewer is a cross-platform, secure, and fast remote desktop application that allows you to access and control computers remotely. Built from the ground up in Rust for maximum performance and security.

## ✨ Features

### 🔒 Security & Privacy
- **End-to-end encryption** using AES-256-GCM
- **Zero-knowledge architecture** - we can't see your sessions
- **Self-hosted option** - run your own relay servers
- **No telemetry** - your privacy is respected

### 🚀 Performance
- **QUIC protocol** for ultra-fast connections
- **Hardware-accelerated** screen capture
- **Adaptive quality** - automatically adjusts to network conditions
- **Low latency** input forwarding

### 🌐 Cross-Platform
- **Windows** (7, 8, 10, 11)
- **Linux** (Ubuntu, Debian, Fedora, Arch, etc.)
- **macOS** (10.14+)

### 📋 Core Functionality
- **Remote desktop control** - Full mouse and keyboard control
- **Screen sharing** - View remote screens in real-time
- **File transfer** - Drag & drop files between computers
- **Clipboard sync** - Share clipboard content
- **Multi-monitor support** - Access all connected displays
- **Session recording** - Record remote sessions (optional)

### 🛠️ Advanced Features
- **Unattended access** - Connect without user interaction
- **Custom resolutions** - Optimize for your network
- **Wake-on-LAN** - Wake up sleeping computers
- **Chat** - Built-in text chat during sessions
- **Voice chat** - Talk while sharing screens (planned)

## 🚀 Quick Start

### Installation

#### From Releases (Recommended)
Download the latest release for your platform from the [Releases page](https://github.com/yourusername/freeviewer/releases).

#### From Source
```bash
# Clone the repository
git clone https://github.com/yourusername/freeviewer.git
cd freeviewer

# Build and run
cargo run --release
```

### Usage

1. **Start FreeViewer** on both computers
2. **Generate ID** - Each computer gets a unique ID
3. **Connect** - Enter the remote computer's ID
4. **Authenticate** - Enter the password shown on remote screen
5. **Control** - You're now connected!

## 🏗️ Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Client GUI    │    │  Relay Server   │    │   Remote Host   │
│                 │◄──►│                 │◄──►│                 │
│ • Control UI    │    │ • NAT traversal │    │ • Screen capture│
│ • Display       │    │ • Connection    │    │ • Input handler │
│ • Input         │    │   routing       │    │ • File server   │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

### Components

- **Client** (`src/client/`) - GUI application for controlling remote computers
- **Host** (`src/host/`) - Background service that allows incoming connections
- **Daemon** (`src/daemon/`) - System service for unattended access
- **Protocol** (`src/protocol/`) - Network protocol implementation
- **Security** (`src/security/`) - Encryption and authentication
- **Capture** (`src/capture/`) - Screen capture and input simulation

## 🔧 Development

### Prerequisites
- Rust 1.70+ 
- Platform-specific dependencies:
  - **Windows**: Visual Studio Build Tools
  - **Linux**: X11 development libraries
  - **macOS**: Xcode command line tools

### Building
```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run with logging
RUST_LOG=debug cargo run
```

### Testing
```bash
# Run all tests
cargo test

# Run specific test
cargo test protocol::tests
```

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

### Development Roadmap
- [ ] Basic remote desktop functionality
- [ ] File transfer system
- [ ] Mobile apps (Android/iOS)
- [ ] Web client
- [ ] Voice chat integration
- [ ] Session recording
- [ ] Plugin system

## 📄 License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by TeamViewer, AnyDesk, and other remote desktop solutions
- Built with amazing Rust libraries from the community
- Special thanks to all contributors and testers

## 📞 Support

- 🐛 **Bug Reports**: [GitHub Issues](https://github.com/yourusername/freeviewer/issues)
- 💬 **Discussions**: [GitHub Discussions](https://github.com/yourusername/freeviewer/discussions)
- 📧 **Email**: support@freeviewer.org

---

**⭐ Star this repository if you find FreeViewer useful!**
