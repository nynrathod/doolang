# Doo Programming Language

[![Rust](https://img.shields.io/badge/Made%20with-Rust-orange)](https://www.rust-lang.org/)
[![LLVM](https://img.shields.io/badge/LLVM-blueviolet)](https://llvm.org/)
[![Native](https://img.shields.io/badge/Compiles%20to-Native-green)](https://llvm.org/)

Doo is a statically-typed, compiled programming language built in Rust + LLVM, designed for building production APIs quickly and safely. It uses automatic memory management via reference counting.

> **Want to contribute? See [CONTRIBUTING.md](CONTRIBUTING.md)**
>
> **For testing and development, see [TEST.md](TEST.md)**

## 🚀 Features

- **Static Type System**: Compile-time type checking with type inference
- **Automatic Memory Management**: Reference counting for data types
- **Data Types**: Integers, Float, Strings, Booleans, Arrays, Maps, and Tuples
- **Native Compilation**: Compiles to standalone executables using clang/lld

## 🔧 Installation

**Download the latest `doo` binary from the [Releases](https://github.com/nynrathod/doolang/releases) page as per your operating system.**

Your downloaded file will usually be saved in your Downloads folder. Please rename file(you will get in format doo-[os-name]-x.x.x) to **doo**. Unzip the downloaded file.

Then, follow the steps below for your operating system:

### Windows

```powershell
# Move doo.exe to a folder (e.g., D:\doo\) and add to PATH
[Environment]::SetEnvironmentVariable("Path", $env:Path + ";D:\doo", [EnvironmentVariableTarget]::User)
```

### Linux / macOS

```sh
# Install clang (required for linking)
# Linux: sudo apt install clang
# macOS: xcode-select --install

chmod +x ~/Downloads/doo
mv ~/Downloads/doo ~/.local/bin/
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc && source ~/.bashrc
```

```
Note: `~/.bashrc` for Linux bash, `~/.bash_profile` for macOS bash
```

- **For zsh (Linux or macOS):**
  `sh
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc
source ~/.zshrc
`

**Verify:** `doo --help`

---

## 🎯 Quick Start

Create your first Doo program:

```rust
// main.doo
fn main() {
    let message: Str = "Hello, doo!";
    print(message);
}
```

Place your `main.doo` file in a project directory and run:

```bash
# Compile and run your program
doo run
```

That's it! Your program compiles to a native executable and runs immediately.

## 🌐 Language Overview

### Variables

```rust
let name = "Doo";           // Type inferred
let age: Int = 25;          // Explicit type
let mut count = 0;          // Mutable
```

### 📊 Data Types

| Type     | Example     |
| -------- | ----------- |
| `Int`    | `42`        |
| `Float`  | `3.14`      |
| `Str`    | `"hello"`   |
| `Bool`   | `true`      |
| `[T]`    | `[1, 2, 3]` |
| `{K: V}` | `{"a": 1}`  |

### Functions

```rust
fn Add(a: Int, b: Int) -> Int {
    return a + b;
}
```

### Structs & Methods

```rust
struct User {
    name: Str,
    age: Int,
}

fn User.isAdult(self) -> Bool {
    return self.age >= 18;
}

fn main() {
    let user = User { name: "Alice", age: 25 };
    print(user.isAdult());  // true
}
```

### Enums

```rust
enum Status { Active, Pending, Done }

let s = Status::Active;
```

### Loops

```rust
// Range
for i in 0..5 { print(i); }

// Array with index
for idx, val in [10, 20, 30] { print(idx, val); }

// Map
for key, val in {"a": 1, "b": 2} { print(key, val); }
```

### Error Handling

```rust
fn divide(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}

fn main() {

    // Single line auto error propagation via ?
    let result, err = divide(10, 2)?;

    // or
    // Manual handling
    // let result, err = divide(10, 2);
    // if err != nil {
    //     print("Error:", err);
    // } else {
    //     print("Result:", result);
    // }
}
```

### JSON

```rust
let data = {"name": "Doo", "version": 1};
let json = JSON.stringify(data);
let parsed: {Str: Int} = JSON.parse(json);
```

### 📦 Modules

```rust
myproject/
├── main.doo
└── utils/
    └── Math.doo
```

```rust
// utils/Math.doo
fn Add(a: Int, b: Int) -> Int { return a + b; }

// main.doo
import utils::Math::Add;

fn main() {
    print(Add(2, 3));
}
```

---

## 📂 Examples

See the [`examples/`](examples/) folder for complete projects:

- See the **[`examples folder`](doo/examples)** for practical sample projects;
- Explore all features raw implementation in the **[`dev_test`](doo/tests/dev_test)**

## 📜 License

This project is licensed under the MIT License

## 🙏 Acknowledgments

- **LLVM Project**: For the powerful backend infrastructure
- **Rust Community**: For inspiration and excellent tooling
- **Programming Language Design Community**: For theoretical foundations

---

**Happy coding with Doo! 🚀**

> **Want to contribute? See [CONTRIBUTING.md](CONTRIBUTING.md)**
>
> **For testing and development, see [TEST.md](TEST.md)**
