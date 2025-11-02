# Doo Programming Language

[![Rust](https://img.shields.io/badge/Made%20with-Rust-orange)](https://www.rust-lang.org/)
[![LLVM](https://img.shields.io/badge/LLVM-blueviolet)](https://llvm.org/)
[![Native](https://img.shields.io/badge/Compiles%20to-Native-green)](https://llvm.org/)

Doo is a statically-typed, compiled programming language with a Rust-inspired syntax designed for simplicity and performance. It features automatic memory management through reference counting, a rich type system, and compiles to standalone native executables using clang and lld.


## 🚀 Features

- **Static Type System**: Compile-time type checking with type inference
- **Automatic Memory Management**: Reference counting for data types
- **Rich Data Types**: Integers, strings, booleans, arrays, maps, and tuples
- **Module System**: Organize code with a hierarchical import system
- **Control Flow**: Conditional statements, for loops, and range iteration
- **Function System**: First-class functions with parameter and return type annotations
- **Native Compilation**: Compiles to standalone executables using clang/lld



## 🔧 Installation

**Download the latest `doo` binary from the [Releases](https://github.com/nynrathod/doolang/releases) page as per your operating system.**

Your downloaded file will usually be saved in your Downloads folder. Please rename file(you will get in format doo-[os-name]-x.x.x) to **doo**, in windows keep .exe at end

Then, follow the steps below for your operating system:


### Windows


1. **Move the downloaded `doo.exe` to a folder of your choice** (e.g., `D:\doo\`).

2. **Add that folder to your PATH** so you can run `doo` from any terminal:

   Open **PowerShell** and run:
   ```powershell
   [Environment]::SetEnvironmentVariable("Path", $env:Path + ";D:\doo", [EnvironmentVariableTarget]::User)
   ```
   *(Restart your terminal after running this to use the new PATH.)*

4. **Verify installation:**
   ```cmd
   doo --help
   ```
---

### Linux & macOS

1. **Install Clang** (required for linking):
   - **Linux:**
     ```sh
     sudo apt update
     sudo apt install clang
     ```
   - **macOS:**
     ```sh
     xcode-select --install
     ```

2. **Make the binary executable, move it to your user bin, and add to your PATH (if needed):**
   ```sh
   chmod +x ~/Downloads/doo
   mkdir -p ~/.local/bin
   mv ~/Downloads/doo ~/.local/bin/doo
   ```
   - Add to your PATH if not already:
     - **For Linux bash:**
       ```sh
       echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc
       source ~/.bashrc
       ```
     - **For macOS bash:**
       ```sh
       echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bash_profile
       source ~/.bash_profile
       ```
     - **For zsh (Linux or macOS):**
       ```sh
       echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.zshrc
       source ~/.zshrc
       ```
     *(Choose the config file that matches your shell and OS: `~/.bashrc` for Linux bash, `~/.bash_profile` for macOS bash, `~/.zshrc` for zsh.)*

3. **Verify installation:**
   ```sh
   doo --help
   ```

---

## 🚀 Usage

- **Navigate to your project root (where `main.doo` is located):**
  ```sh
  cd /path/to/your/project
  ```
- **Compile and run your project:**
  ```sh
  doo run
  ```

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

### Design Philosophy

Doo combines the expressiveness of high-level languages with the performance of compiled languages, focusing on **readability** with clean and simple syntax that's easy to learn.


## 📊 Data Types

Doo provides a rich set of built-in types:

### Primitive Types

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 32-bit signed integer | `42`, `-10` |
| `Str` | UTF-8 string | `"Hello, World!"` |
| `Bool` | Boolean value | `true`, `false` |

### Collection Types

| Type | Description | Example |
|------|-------------|---------|
| `[T]` | Array of type T | `[1, 2, 3]`, `["a", "b", "c"]` |
| `{K: V}` | Map with key type K and value type V | `{"name": "Alice", "age": 30}` |

### Complex Types
##### Only support for loop as of now

| Type | Description | Example |
|------|-------------|---------|
| `(T1, T2)` | Tuple of multiple types | `(42, "hello", true)` |

## 📝 Syntax Guide

### Variables

Variables are declared with `let` and can be mutable with `mut`:

```rust
let immutable: Int = 42;           // Immutable variable with explicit type
let mut changeable: Str = "hello"; // Mutable variable with explicit type
let inferred = 42;                 // Type inferred from value
```

### Functions

Functions use PascalCase for public functions and camelCase for private ones:

```rust
fn AddNumbers(a: Int, b: Int) -> Int {
    return a + b;
}

fn privateHelper(x: Str) -> Str {
    return x;
}

fn main() {
    let result = AddNumbers(5, 10);
    print(result);
}
```

### Control Flow

#### Conditional Statements

```rust
let condition: Bool = true;

if condition {
    print("Condition is true");
} else {
    print("Condition is false");
}

// Nested conditions
if condition {
    if result > 10 {
        print("Large result");
    } else {
        print("Small result");
    }
}
```

#### For Loops

```rust
// Range iteration (exclusive)
for i in 0..5 {
    print(i); // Prints 0, 1, 2, 3, 4
}

// Range iteration (inclusive)
for i in 0..=5 {
    print(i); // Prints 0, 1, 2, 3, 4, 5
}

// Array iteration
let numbers: [Int] = [1, 2, 3, 4, 5];
for n in numbers {
    print(n);
}

// Map iteration (key-value pairs)
let scores: {Str: Int} = {"Alice": 95, "Bob": 87};
for (name, score) in scores {
    print(name, ":", score);
}
```

### Expressions and Operators

```rust
// Arithmetic operators
let sum = 5 + 10;
let difference = 20 - 5;
let product = 6 * 7;
let quotient = 100 / 5;

// Comparison operators
let is_equal = 5 == 5;
let not_equal = 5 != 10;
let greater = 10 > 5;
let less_equal = 5 <= 10;


// String concatenation
let greeting = "Hello, " + "World!";
```

## 📦 Module System

Doo uses a hierarchical module system with `::` separators:

```
myproject/
├── main.doo              # Entry point
├── models/
│   └── User.doo         # User-related functions
└── http/
    └── Client.doo       # HTTP client functions
```

### Import Syntax

```rust
// main.doo
import models::User::CreateUser;
import http::Client::FetchUser;

fn main() {
    let user = CreateUser("Alice", 30);
    let data = FetchUser(1);
    print(user);
    print(data);
}
```

```rust
// models/User.doo
fn CreateUser(name: Str, age: Int) -> Str {
    return "User: " + name + ", Age: " + age;
}
```

```rust
// http/Client.doo
fn FetchUser(id: Int) -> Str {
    return "User data for ID: " + id;
}
```

## 💡 Examples

### Basic Calculator

```rust
// math.doo
fn Add(a: Int, b: Int) -> Int {
    return a + b;
}

fn Multiply(a: Int, b: Int) -> Int {
    return a * b;
}

// main.doo
import math::Add;
import math::Multiply;

fn main() {
    let x: Int = 10;
    let y: Int = 5;

    let sum = Add(x, y);
    let product = Multiply(x, y);

    print("Sum:", sum);
    print("Product:", product);
}
```

### Data Processing

```rust
fn main() {

    // Array creation and iteration
    let numbers: [Int] = [1, 2, 3, 4, 5];

    print("Numbers:", numbers);

    // Simple for loop iteration
    for item in numbers {
        print("Number:", item);
    }

    // Map creation and access
    let scores: {Str: Int} = {"Alice": 95, "Bob": 87, "Charlie": 92};

    print("Scores:", scores);

    // String operations
    let name: Str = "Doo";
    let greeting = "Hello, " + name + "!";

    print(greeting);
}
```

### File Organization Example

```
school/
├── main.doo
├── students/
│   └── Student.doo
└── grades/
    └── Calculator.doo
```

```rust
// main.doo
import students::Student::CreateStudent;
import grades::Calculator::CalculateGPA;

fn main() {
    let student = CreateStudent("Alice", 20);
    let gpa = CalculateGPA([95, 87, 92, 88]);

    print(student);
    print("GPA:", gpa);
}
```

## 📂 More Examples

- See the **[examples folder](doo\examples)** for practical sample projects.
- Explore the **[valid test programs](doo\tests\programs\valid)** for a wide range of language features and usage patterns.
- Also check out the **[multifile test project](doo\tests\multifile_test_project)** for examples of multi-file projects and advanced import/module usage.

## 📜 License

This project is licensed under the MIT License

## 🙏 Acknowledgments

- **LLVM Project**: For the powerful backend infrastructure
- **Rust Community**: For inspiration and excellent tooling
- **Programming Language Design Community**: For theoretical foundations

---

**Happy coding with Doo! 🚀**

## 📚 Additional Resources

- **[CONTRIBUTING.md](CONTRIBUTING.md)**: Guide for developers who want to contribute to Doo
