# Sphere: The Universal Compute Runtime

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

Sphere is a next-generation, sandboxed computational runtime designed to solve the "it works on my machine" problem once and for all. It allows you to define any computational process, along with its dependencies, in a simple file and execute it identically on any machine—from a cloud server to an Android phone.

**It's like Docker, but 100x lighter, more secure by default, and without the daemons.**

---

### The Vision

In today's world, software is plagued by two fundamental problems:
1.  **The Environment Problem:** Code is fragile and deeply tied to the environment it runs in (OS, libraries, paths).
2.  **The Trust Problem:** We run scripts and install packages with little to no guarantee of what they can actually do on our system.

Sphere tackles these head-on by treating computation like a law of physics. A Sphere process is defined by a `.sphere` file, which is a portable, self-contained "gene" for a computation. The `sphere` runtime acts as a universal "cell" that can execute this gene in a pristine, isolated sandbox, anywhere.

### Key Features

*   **Universal Portability:** A `.sphere` file that runs on your Linux server will run *identically* on `termux` on your phone. No more surprises.
*   **Secure by Default:** Spheres run with zero permissions by default. They must explicitly declare what files they need to access. (Note: True `chroot` sandboxing is on the roadmap).
*   **Simple, Declarative Format:** Define your processes in a clean, human-readable TOML file.
*   **Composable Dependencies:** Build complex workflows by linking simple Spheres together.
*   **Blazingly Fast & Lightweight:** Written in Rust for performance and a minimal footprint.

### Getting Started

#### 1. Installation

Currently, you can install Sphere by building from source.

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone the repository
git clone https://github.com/Nakadra/sphere-runtime.git
cd sphere-runtime

# Build the release binary
cargo build --release

# Place the binary in your PATH
cp target/release/sphere $PREFIX/bin/sphere
```

#### 2. Your First Sphere

Create a file named `hello.sphere`:
```toml
entrypoint = "echo 'Hello, from my first Sphere!'"
```

Run it:
```bash
sphere hello.sphere
```

### The Roadmap

Sphere is a young but ambitious project. Our goal is to build the foundational layer for the next generation of software. Here's where we're going:

*   [x] **Phase 0: Core Runtime MVP**
*   [ ] **Phase 1: Community & SphereHub MVP** (Public registry for sharing Spheres)
*   [ ] **Phase 2: True Sandboxing** (Implementing `chroot` for full imprisonment)
*   [ ] **Phase 3: The Global Grid** (A decentralized network for running Spheres)

### Contributing

This project is built by the community, for the community. We welcome all contributions. Please see our `CONTRIBUTING.md` for guidelines.

---

*Built with ❤️ in Nairobi, Kenya.*
