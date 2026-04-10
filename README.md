# Nyx

-----

## Nyx Language — Compilation & Execution Pipeline

### Overview

Nyx source code travels through a multi-stage pipeline before becoming program output. Every stage has a single responsibility. Errors are caught as early as possible — ideally at compile time — so that runtime is as safe and predictable as possible.

-----

### Stage 1 — Lexer

Turns raw `.nyx` source text into a flat stream of tokens.

- Handles whitespace-sensitive operator disambiguation
- Recognizes `%`, `#`, `!` prefixes
- Validates identifier characters
- Hard errors on malformed tokens before anything else proceeds

-----

### Stage 2 — `%make` Pass

Extracts and executes compile-time configuration before any other analysis.

- Validates `%make` structure and known directives
- Merges top-level `#import` / `#use` / `#def` shorthands into `%make` lists
- Checks `%logic-%make` before allowing control flow inside `%make`
- Fires `%when-compile` `%rust` hooks

-----

### Stage 3 — Parser

Turns the token stream into an Abstract Syntax Tree (AST).

- Validates scope rules and the single-return-value-per-scope rule
- Enforces `#` prefix at depth 0; hard errors on `#` inside inner blocks
- Warns on missing `#` at top level
- Hard errors on malformed expressions immediately

-----

### Stage 4 — Directive Validator

Checks every `%`-prefixed token against the known directive list.

- Warns on unknown `%` identifiers
- Suggests the closest known directive by name similarity
- Hard errors on `%` directives used in the wrong context (e.g. `%make` directives outside `%make`)

-----

### Stage 5 — Import Cycle Detector

Checks the full `%import` graph for circular dependencies before type inference can be poisoned.

- Hard errors on cycles (e.g. `a` imports `b` imports `a`)
- Reports the full cycle chain in the error message

-----

### Stage 6 — Type Inference

Walks the AST and infers types for all expressions and bindings.

- Resolves generic type parameters at call sites
- Infers return types from bare expressions
- Hard errors on ambiguous types that cannot be resolved

-----

### Stage 7 — Type Checker

Verifies all inferred types are consistent and valid across the program.

- Zero implicit coercion — every type mismatch is a hard error
- Ensures function signatures match their call sites
- Validates `std::convert` family compatibility
- Hard errors on cross-family conversion without `%hard`
- Ensures generics are used consistently across all call sites

-----

### Stage 8 — Strict Inference Validator

A second pass on type inference results to catch any remaining ambiguity.

- Ensures no type was inferred as ambiguous or unknown
- Forces explicit annotation if inference produces multiple candidates
- Hard errors on any unresolved type variable

-----

### Stage 9 — Numeric Safety Checker

Catches numeric errors at compile time wherever possible.

- Hard errors on compile-time integer overflows and underflows (e.g. `let %i8 x = 200`)
- Hard errors on obvious division by zero (e.g. `x / 0`)
- Hard errors on modulo by zero
- Warns on arithmetic that could overflow at runtime
- Warns on narrowing conversions (e.g. `%i64` → `%i8`)
- Tracks numeric ranges through branches where possible
- Emits runtime overflow guards for non-static arithmetic

-----

### Stage 10 — Ownership Checker

Validates the ownership model across the entire program.

- Catches use-after-move errors
- Tracks ownership transfer across function calls
- Validates moved values are never read again
- Checks drop order is deterministic and correct
- Hard errors on any possibility of double free
- Ensures owned values are always dropped exactly once

-----

### Stage 11 — Borrow Checker

Enforces borrow rules across all scopes and function boundaries.

- Hard errors on simultaneous `&x` and `&mut x`
- Hard errors on two `&mut x` at the same time
- Ensures borrows don’t outlive their owner
- Validates borrows across function call boundaries

-----

### Stage 12 — Lifetime Checker

Ensures references never outlive the data they point to.

- Catches dangling references before they reach the VM
- Hard errors on returning a reference to a local variable
- Ensures no reference ever points to a dropped value
- Validates borrow lifetimes across function boundaries

-----

### Stage 13 — Drop Order Validator

Verifies the exact order values are dropped in every scope.

- Ensures drop order is always reverse of declaration order
- Hard errors if a borrow outlives the drop of its owner
- Validates drops across all possible code paths

-----

### Stage 14 — Double Free Detector

Dedicated pass to ensure no value is ever dropped twice.

- Tracks every possible execution path through the program
- Hard errors if any path could result in a double drop
- Validates moves never leave a dangling original binding

-----

### Stage 15 — Mutability Checker

Ensures immutability is airtight across the program.

- Catches mutation of non-`%mut` variables
- Catches mutation through immutable borrows
- Validates `&mut` is only used on `%mut` bindings

-----

### Stage 16 — Initialization Checker

Ensures every variable is initialized before use.

- Hard errors on reading an uninitialized binding
- Tracks initialization through all branches
- If only one branch initializes `x`, reading `x` after is a hard error
- Ensures function parameters are always initialized at the call site

-----

### Stage 17 — Exhaustiveness Checker

Validates that match statements and error handling are complete.

- Hard errors on `match` statements missing arms
- Suggests missing arms in the error message
- Validates all `Result<T>` values are handled or propagated
- Hard errors on ignored `Result` values — every error must be handled

-----

### Stage 18 — Forced Error Handling Validator

A dedicated pass ensuring no `Result` is ever silently discarded.

- Every function returning `Result<T>` must be matched, unwrapped, or `?`’d
- Hard errors on calling a `Result`-returning function without handling it
- Ensures `panic` is never used where `Result` is more appropriate

-----

### Stage 19 — Reachability Checker

Detects code that can never execute.

- Hard errors on two bare expressions in the same scope
- Warns on code that can never be reached

-----

### Stage 20 — Control Flow Analyzer

Ensures every function returns on every possible code path.

- Hard errors on functions that might not return on some branches
- Validates loop `break` values are consistent types
- Ensures `?` is only used inside `Result`-returning functions
- Hard errors on infinite loops with no break path

-----

### Stage 21 — Panic Analyzer

Makes panics visible and traceable through the call graph.

- Warns if a function can transitively panic
- Suggests wrapping in `Result` if the panic is recoverable
- In `%strict` mode: panics in non-panic-annotated functions are hard errors

-----

### Stage 22 — Dead Code Analyzer

Warns on unused code and imports.

- Warns on unused variables, functions, and imports
- Warns on `%use`’d items that are never called
- Warns on `%def`’d libraries that are never used
- Respects `%suppress-warnings`

-----

### Stage 23 — Narrowing Checker

Validates array access safety at compile time.

- Hard errors on `arr[0]` — arrays are 1-indexed
- Catches statically obvious out-of-bounds indices
- Warns on runtime-dynamic indices that could go out of bounds
- Emits runtime bounds check guards for dynamic indices

-----

### Stage 24 — Resource Safety Checker

Ensures all resources are always closed on every code path.

- Hard errors if a resource-owning value goes out of scope unclosed
- Enforces a drop handler on all resource types
- Validates resources are closed on all paths including error paths and early returns

-----

### Stage 25 — Concurrency Safety Checker

*(Reserved — activates when threads are added to Nyx)*

- Validates no data races via ownership rules
- Ensures shared data is always behind safe concurrency primitives
- Hard errors on sending non-thread-safe values across threads

-----

### Stage 26 — `#rust` Isolator

Sandboxes all `#rust {}` blocks and `%def` linkages from the Nyx safety model.

- Runs `rustc` on every `#rust` block in isolation before proceeding
- Hard errors if any `#rust` block fails to compile
- Ensures raw Rust cannot violate Nyx ownership rules
- Validates all values crossing the Nyx / `#rust` boundary are safe types
- Verifies `%def` ABI compatibility before bytecode emission
- Validates `%when-run` and `%when-compile` `%rust` blocks compile cleanly
- Wraps all `#rust` return values in `Result` — Rust panics become Nyx errors
- Audits `#rust` blocks for `unsafe` Rust — warns on every `unsafe` usage

-----

### Stage 27 — Compiler

Translates the fully checked AST into Nyx bytecode.

- Emits `MOV`, `DROP`, `BORROW`, `RELEASE`, `CALL`, `RET` instructions
- Inlines all `%import` / `%use` functions at call sites
- Bakes the ownership table and register assignments into bytecode
- Emits runtime guards for overflow, bounds, and OOM checks

-----

### Stage 28 — Bytecode Verifier

Final sanity check on emitted bytecode before it is ever run.

- Ensures all jumps land on valid instructions
- Ensures no register is read before being written
- Ensures the ownership table is internally consistent
- Ensures all emitted runtime guards are present and correct
- Hard errors on any malformed bytecode — bad bytecode never runs

-----

### Stage 29 — Bytecode (`.nyxb`)

The output of compilation: a flat binary instruction stream.

- Register assignments baked in
- Ownership table baked in
- Runtime guards baked in

-----

### Stage 30 — VM Startup

Initializes the VM before execution begins.

- Initializes the register file and ownership tracker
- Loads bytecode into instruction memory
- Validates bytecode signature before executing anything
- Allocates initial memory with OOM detection
- Fires `%when-run` `%rust` hooks

-----

### Stage 31 — VM Execution

Executes bytecode instruction by instruction.

- Enforces ownership rules at runtime as a final safety net
- Handles `Result` propagation and panic unwinding
- Manages deterministic drops at scope boundaries
- Tracks resource handles and forces close on scope exit

-----

### Stage 32 — VM Numeric Runtime Guard

Catches numeric errors that could not be resolved at compile time.

- Checks every arithmetic operation for overflow and underflow
- Division by zero → `err()`, not a crash
- Modulo by zero → `err()`, not a crash
- In debug mode: traps immediately with source location
- In release mode: returns `err()` and unwinds gracefully

-----

### Stage 33 — VM Memory Runtime Guard

Monitors memory safety at runtime.

- Monitors heap allocation for OOM conditions — OOM returns `err()`, never a crash
- Detects stack overflow before it happens via a stack depth counter
- Stack overflow → graceful `err()`, not a segfault
- Validates every `DROP` is for a live owned value

-----

### Stage 34 — VM Bounds Runtime Guard

Validates array access safety at runtime.

- Checks every array access against array length
- Out of bounds → `err()`, not a crash
- Reports exact index and array size in the error message

-----

### Stage 35 — VM Unexpected Error Trap

The absolute last line of defense. Nothing escapes this.

- Catches any error not handled by the above guards
- Every unhandled VM state produces a structured error report
- Never produces an uncontrolled crash under any circumstance
- Writes a full crash report to stderr including:
  - Register state
  - Ownership table
  - Instruction pointer
  - Source line
- Always exits cleanly with a non-zero code — never a raw segfault

-----

### Summary Table

|Stage|Name                      |Hard Errors                |Warnings          |
|-----|--------------------------|---------------------------|------------------|
|1    |Lexer                     |Malformed tokens           |—                 |
|2    |`%make` Pass              |Bad directives             |—                 |
|3    |Parser                    |Scope violations, bad AST  |Missing `#`       |
|4    |Directive Validator       |Wrong context              |Unknown `%`       |
|5    |Import Cycle Detector     |Cycles                     |—                 |
|6    |Type Inference            |Ambiguous types            |—                 |
|7    |Type Checker              |Type mismatches            |—                 |
|8    |Strict Inference Validator|Unresolved types           |—                 |
|9    |Numeric Safety Checker    |Overflow, div/0            |Runtime risk      |
|10   |Ownership Checker         |Use-after-move, double free|—                 |
|11   |Borrow Checker            |Borrow conflicts           |—                 |
|12   |Lifetime Checker          |Dangling references        |—                 |
|13   |Drop Order Validator      |Bad drop order             |—                 |
|14   |Double Free Detector      |Double drop                |—                 |
|15   |Mutability Checker        |Illegal mutation           |—                 |
|16   |Initialization Checker    |Uninitialized read         |—                 |
|17   |Exhaustiveness Checker    |Incomplete match           |—                 |
|18   |Forced Error Handling     |Ignored Result             |—                 |
|19   |Reachability Checker      |Two bare expressions       |Unreachable code  |
|20   |Control Flow Analyzer     |Missing return path        |—                 |
|21   |Panic Analyzer            |—                          |Transitive panics |
|22   |Dead Code Analyzer        |—                          |Unused code       |
|23   |Narrowing Checker         |arr[0], bad bounds         |Dynamic index risk|
|24   |Resource Safety Checker   |Unclosed resource          |—                 |
|25   |Concurrency Safety Checker|Data races *(future)*      |—                 |
|26   |`#rust` Isolator          |Bad Rust, ABI mismatch     |`unsafe` usage    |
|27   |Compiler                  |—                          |—                 |
|28   |Bytecode Verifier         |Malformed bytecode         |—                 |
|29   |Bytecode `.nyxb`          |—                          |—                 |
|30   |VM Startup                |Bad signature, OOM         |—                 |
|31   |VM Execution              |—                          |—                 |
|32   |VM Numeric Guard          |→ `err()`                  |—                 |
|33   |VM Memory Guard           |→ `err()`                  |—                 |
|34   |VM Bounds Guard           |→ `err()`                  |—                 |
|35   |VM Unexpected Error Trap  |Structured report          |—                 |# Nyx

**A Safe, Fast, Experimental Programming Language**
*Language Specification & Design Reference — v0.2*

-----

## Overview

Nyx is a statically typed, ownership-based programming language that compiles to custom bytecode and runs on the Nyx Virtual Machine (NyxVM). It is designed to be simultaneously beginner-friendly and production-safe — offering Rust-like memory guarantees without the complexity tax.

Nyx is built around three core values:

- **Simple** — readable syntax, friendly errors, guided best practices
- **Safe** — 35-stage compiler pipeline, no null, no undefined behavior, no raw crashes
- **Fast** — ownership-based memory, no garbage collector, green thread runtime

The compiler is called `nyxc`. The virtual machine is called `nyxvm`. Source files use the `.nyx` extension. Compiled bytecode uses `.nyxb`.

-----

## Design Philosophy

### 1-Indexed Arrays

Arrays in Nyx start at index 1, not 0. This eliminates a common class of off-by-one errors. The compiler hard-errors on `arr[0]` accesses.

### % Prefix Convention

Everything that communicates with the compiler uses the `%` prefix. Types, directives, literals — if it tells the compiler something, it starts with `%`. This creates a clear visual distinction between user code and compiler instructions.

### No Null

There is no `null`, `nil`, or `None` in Nyx. Absence is represented by `%void` (which is falsy). Functions that can fail return `%Result<T>`. This eliminates an entire category of runtime errors by construction.

### Friendly Errors

The compiler prefers warnings over errors where safety allows. Missing a `#` prefix at the top level? Warning, not a crash. Using an unknown `%` directive? Warning with a suggestion. Hard errors are reserved for genuinely unsafe situations.

### Everything is `%nyx(){}`

At the compiler level, all code ultimately compiles down to `%nyx(){}` blocks — first-class callable code units. Functions, class methods, lifecycle hooks, and async tasks are all sugar over this primitive.

-----

## File Structure

Every Nyx source file follows the same structure:

```nyx
// 1. Optional %make block (compile-time config)
#fn %make() {
    let %target = "release";
    let %import = [math, geometry];
    let %use = [math::add];
};

// 2. Top-level declarations (must be prefixed with #)
#class Point { ... };
#fn %pub calculate() -> %f64 { ... };

// 3. Entry point
#fn main() -> %void {
    // program starts here
};
```

The entry point defaults to `main` but can be overridden via `%entry` in `%make`.

-----

## Scope Rules

- Every `{}` block is its own independent scope
- Every statement and closing `}` ends with `;` unless it is a return value
- Each scope may have at most **one** bare expression (the return value) — hard error if violated
- Depth 0 (top of file) is the Main scope — declarations must use `#` prefix
- Missing `#` at depth 0 → warning, proceeds normally
- Using `#` inside an inner block → hard error

```nyx
// Valid scope with return value
#fn add(x: %i32, y: %i32) -> %i32 {
    let z = x + y;
    z                // bare expression = return value, no semicolon
};

// Hard error — two bare expressions in same scope
#fn bad() -> %i32 {
    x               // ERROR
    y               // ERROR
};
```

-----

## Directive Reference

Directives use the `%` prefix and are checked against a fixed hardcoded list in the compiler. Using an unknown `%` token produces a warning. Using `%` in the wrong context is a hard error.

### Variable Modifiers

|Directive|Meaning                               |
|---------|--------------------------------------|
|`%mut`   |Mutable binding                       |
|`%pub`   |Public visibility (private by default)|

### Types

|Directive                       |Meaning                                           |
|--------------------------------|--------------------------------------------------|
|`%i8` / `%i16` / `%i32` / `%i64`|Signed integers                                   |
|`%u8` / `%u16` / `%u32` / `%u64`|Unsigned integers                                 |
|`%f32` / `%f64`                 |Floating point                                    |
|`%str`                          |String                                            |
|`%bool`                         |Boolean                                           |
|`%char`                         |Character                                         |
|`%void`                         |No value (also falsy)                             |
|`%rust`                         |Raw Rust code block                               |
|`%Result<T>`                    |Sugar for `[%bool, T]` — desugared at compile time|
|`%nyx(){}`                      |First-class Nyx code block                        |
|`%rust(){}`                     |First-class Rust code block                       |

### Boolean Literals

|Literal |Value                                              |
|--------|---------------------------------------------------|
|`%true` |Boolean true                                       |
|`%false`|Boolean false                                      |
|`%void` |Also falsy — equivalent to false in boolean context|

### `%make` Directives

Only valid inside `#fn %make()`. Using them outside `%make` is a hard error.

|Directive           |Type         |Meaning                                          |
|--------------------|-------------|-------------------------------------------------|
|`%make`             |—            |Marks the compile-time config function           |
|`%logic-%make`      |`%bool`      |Enables control flow inside `%make`              |
|`%suppress-warnings`|`%bool`      |Silences all compiler warnings                   |
|`%target`           |`%str`       |`"debug"` or `"release"`                         |
|`%entry`            |`%str`       |Override the entry point function name           |
|`%strict`           |`%bool`      |Treat warnings as hard errors                    |
|`%hard`             |`[fn, ...]`  |List of functions with restrictions removed      |
|`%when-run`         |`%rust`      |`%rust` code to execute before program runs      |
|`%when-compile`     |`%rust`      |`%rust` code to execute before compilation       |
|`%import`           |`[mod, ...]` |List of Nyx modules to import                    |
|`%use`              |`[path, ...]`|List of specific items from imported modules     |
|`%def`              |`[file, ...]`|List of Rust `.rs` / `.dll` / `.so` files to link|
|`%repl`             |`%bool`      |`%true` launches REPL mode, ignores `main`       |
|`%async`            |—            |Enables the async green thread runtime           |
|`%self`             |`%str` or map|Rename the self reference inside class methods   |

-----

## Variables

Variables are immutable by default. Types are inferred but can be annotated explicitly.

```nyx
let x = 42;                    // immutable, inferred %i64
let %mut y = 3.14;             // mutable, inferred %f64
let %mut %i32 z = 100;         // mutable, explicit type
let %str name = "nyx";         // explicit type, immutable
```

### Mass `let`

Multiple variables can be declared in a single `let` block:

```nyx
let = {
    x = 1,
    y = 2,
    %mut %str name = "hello",
};
```

### `%make` Globals

Variables declared inside `%make` are permanent compile-time globals, baked into the program. They cannot be overridden at runtime. Attempting to rebind protected names like `%true`, `%false`, or `%void` is a hard error.

-----

## Functions

```nyx
#fn add(x: %i32, y: %i32) -> %i32 {
    x + y                      // implicit return
};

#fn greet(name: %str) -> %void {
    print("hello {std::hconvert(%str, name)}");
};
```

- Functions are private by default — use `%pub` to expose them
- Last bare expression is the implicit return value
- Explicit `return x;` is also valid
- `return x;` with a semicolon is a statement, not the implicit return

### Function Modifiers

|Modifier                 |Meaning                                        |
|-------------------------|-----------------------------------------------|
|`#fn %pub foo()`         |Publicly accessible from other modules         |
|`#fn %async foo()`       |Asynchronous function                          |
|`#fn %spawn foo()`       |Always runs in its own green thread when called|
|`#fn %spawn %async foo()`|Threaded and asynchronous                      |

-----

## Classes

Classes bundle data and methods together. Fields are declared in a `create` block. Methods use `%self` to reference the instance.

```nyx
#class Circle {
    create {
        let radius: %f64,
        let color: %str = "red",   // default value
    };

    fn area(%self) -> %f64 {
        3.14159 * %self.radius * %self.radius
    };

    fn %pub describe(%self) -> %str {
        "A {std::hconvert(%str, %self.color)} circle"
    };
};

// Construction
let c = Circle.create { radius = 5.0 };
let area = c.area();
```

The `%self` keyword can be renamed globally in `%make`:

```nyx
#fn %make() {
    let %self = "this";              // all classes use 'this'
    // or per-class:
    let %self = {
        Circle = "this",
        "self",                      // default fallback
    };
};
```

-----

## Control Flow

### if / else

```nyx
if x > 0 {
    "positive"
} else if x < 0 {
    "negative"
} else {
    "zero"
};
```

### while

```nyx
while x > 0 {
    x = x - 1;
};
```

### loop

```nyx
let result = loop {
    if condition {
        break 42;
    };
};
```

### for

```nyx
for i in 1..10 {
    print(arr[i]);
};
```

### match

```nyx
match x {
    0 => "zero",
    1..10 => "small",
    _ => "big",
};
```

Match is exhaustive — missing arms are a hard error.

-----

## Memory Model

Nyx uses an ownership-based memory model. There is no garbage collector. Memory is freed deterministically when a value goes out of scope.

### Ownership

```nyx
let x = 5;          // x owns the value
let y = x;          // ownership moves to y, x is now invalid
print(x);           // ERROR: x was moved
```

### Borrowing

```nyx
let x = 5;
let y = &x;         // immutable borrow — x still owns the value
let z = &mut x;     // ERROR: cannot borrow mutably while immutably borrowed
```

|Rule                            |Allowed?    |
|--------------------------------|------------|
|Multiple `&x` at once           |✅ Yes       |
|One `&mut x`                    |✅ Yes       |
|`&x` and `&mut x` simultaneously|❌ Hard error|
|Two `&mut x` simultaneously     |❌ Hard error|
|Using `x` after move            |❌ Hard error|

-----

## Error Handling

`%Result<T>` is the standard return type for functions that can fail. It is sugar for `[%bool, T]` — an array where index 1 is the success status and index 2 is the value or error message.

```nyx
#fn divide(a: %f64, b: %f64) -> %Result<%f64> {
    if b == 0.0 {
        err("division by zero")     // [%false, "division by zero"]
    } else {
        ok(a / b)                   // [%true, result]
    }
};

// Handling
let result = divide(10.0, 2.0);
if result[1] {
    print("success: {std::hconvert(%str, result[2])}");
} else {
    print("error: {std::hconvert(%str, result[2])}");
};

// ? operator — propagates error immediately
let value = divide(10.0, 0.0)?;    // returns [%false, msg] if err
```

Every `%Result<T>` must be handled, matched, or propagated with `?`. Ignoring a Result is a hard error.

For unrecoverable errors:

```nyx
panic("something went catastrophically wrong");
```

-----

## Arrays

Arrays are 1-indexed. Index 0 is a hard compiler error.

```nyx
let arr = [10, 20, 30, 40, 50];
let first = arr[1];             // = 10
let last = arr[5];              // = 50
arr[0];                         // HARD ERROR

for i in 1..5 {
    print(arr[i]);
};
```

-----

## Code Blocks — `%nyx(){}`

`%nyx(){}` is the atomic unit of all Nyx code. Everything compiles down to it. Use it explicitly when you need a disposable, one-time code block without polluting the namespace.

```nyx
// Immediate invocation
let result = %nyx(x: %i32, y: %i32) -> %i32 {
    x + y
}(3, 4);              // = 7

// Pass as argument
#fn apply(f: %nyx(%i32) -> %i32, val: %i32) -> %i32 {
    f(val)
};

let double = %nyx(x: %i32) -> %i32 { x * 2 };
apply(double, 5);    // = 10
```

The `-> type` annotation makes the return type explicit and statically checked. Without it, the type is inferred.

Other language blocks follow the same syntax:

```nyx
let fast-sqrt = %rust(x: %f64) -> %f64 {
    x.sqrt()
};
```

-----

## Imports & Modules

```nyx
#fn %make() {
    let %import = [math, geometry];
    let %use = [math::add, geometry::distance];
    let %def = ["mylib.rs" as mylib, "fast.dll" as fast];
};
```

- `%import` brings a module into scope — required before `%use`
- `%use` selects specific items and inlines them at call sites
- `%def` links Rust source files or precompiled binaries
- Using `%use` without first `%import`ing the module → hard error

### Naming Conflicts

If two `%use`’d items share a name, the compiler warns and revokes the shorthand for both — forcing full namespace paths:

```nyx
math::add(1, 2);       // required if add conflicts
geometry::add(1, 2);   // required if add conflicts
```

### Visibility

Functions and classes are private by default. Use `%pub` to expose them:

```nyx
#fn %pub add(x: %i32, y: %i32) -> %i32 { x + y };
```

-----

## Rust Interop

Nyx has two mechanisms for calling Rust code.

### Inline `%rust(){}`

```nyx
#fn fast-op(x: %f64) -> %f64 {
    %rust(x: %f64) -> %f64 {
        x.sqrt()
    }(x)
};
```

### `%def` — External Rust Files

Mark Rust functions with `#[nyx_abi]` to expose them to Nyx. The compiler maps Rust types to Nyx types automatically:

```rust
// http.rs
#[nyx_abi]
pub fn fetch(url: &str) -> Result<String, String> {
    // ...
}
```

```nyx
// main.nyx
#fn %make() {
    let %def = ["http.rs" as http];
};

let result = http::fetch("https://example.com")?;
```

### Type Mapping

|Rust Type          |Nyx Type      |
|-------------------|--------------|
|`&str` / `String`  |`%str`        |
|`i8/16/32/64`      |`%i8/16/32/64`|
|`u8/16/32/64`      |`%u8/16/32/64`|
|`f32/64`           |`%f32/64`     |
|`bool`             |`%bool`       |
|`Result<T, String>`|`%Result<T>`  |
|`()`               |`%void`       |

Types with no mapping cannot cross the boundary — hard error.

-----

## Async & Green Threads

Nyx uses green threads managed by NyxVM — lightweight, cheap to spawn, and safe by construction via ownership transfer.

```nyx
#fn %make() {
    let %async = %true;
};

// %async function
#fn %async fetch(url: %str) -> %Result<%str> {
    let response = await net::get(url)?;
    ok(response.body)
};

// %spawn — always runs in its own green thread
#fn %spawn %async process(data: [%i32]) -> %i32 {
    await heavy-calculation(data)
};

// Call and wait
let result = await process(my-data);
```

- `await` waits for both `%async` and `%spawn` functions
- Values are moved into threads — the caller loses ownership
- Ownership is returned via the function return value
- `%nyx(){}` blocks cannot be spawned — only `#fn %spawn` can spawn threads

-----

## Built-in Equation Solver

Nyx has algebraic equation solving built into the language via the `%solve` directive. Solving happens at runtime.

```nyx
// Linear: solve 0 = 2x + 4
let %solve x = 2 * x + 4;     // x = ok(-2.0)

// Quadratic: two solutions returned as array
let %solve x = x * x - 4;     // x = [ok(2.0), ok(-2.0)]
//   x[1] = positive solution
//   x[2] = negative solution

// No solution returns err
let %solve x = x + 1;         // x = err("no solution")

// System of equations
let %solve {
    x + y = 5;
    x - y = 1;
} -> (x, y);
```

-----

## REPL Mode

REPL mode launches an interactive session. Enable it in `%make`:

```nyx
#fn %make() {
    let %repl = %true;
};
```

- When `%repl` is `%true`, `#fn main()` is ignored with a warning
- Variables defined in the session persist between lines
- Bare expressions print their value automatically
- Errors print inline without crashing the session
- The REPL waits for `};` before evaluating a block

-----

## Compilation & Execution Pipeline

Nyx source code passes through 35 distinct stages before producing program output. Every stage has a single responsibility.

|Stage|Name                      |Hard Errors                |Warnings          |
|-----|--------------------------|---------------------------|------------------|
|1    |Lexer                     |Malformed tokens           |—                 |
|2    |`%make` Pass              |Bad directives             |—                 |
|3    |Parser                    |Scope violations, bad AST  |Missing `#`       |
|4    |Directive Validator       |Wrong context              |Unknown `%`       |
|5    |Import Cycle Detector     |Cycles                     |—                 |
|6    |Type Inference            |Ambiguous types            |—                 |
|7    |Type Checker              |Type mismatches            |—                 |
|8    |Strict Inference Validator|Unresolved types           |—                 |
|9    |Numeric Safety Checker    |Overflow, div/0            |Runtime risk      |
|10   |Ownership Checker         |Use-after-move, double free|—                 |
|11   |Borrow Checker            |Borrow conflicts           |—                 |
|12   |Lifetime Checker          |Dangling references        |—                 |
|13   |Drop Order Validator      |Bad drop order             |—                 |
|14   |Double Free Detector      |Double drop                |—                 |
|15   |Mutability Checker        |Illegal mutation           |—                 |
|16   |Initialization Checker    |Uninitialized read         |—                 |
|17   |Exhaustiveness Checker    |Incomplete match           |—                 |
|18   |Forced Error Handling     |Ignored Result             |—                 |
|19   |Reachability Checker      |Two bare expressions       |Unreachable code  |
|20   |Control Flow Analyzer     |Missing return path        |—                 |
|21   |Panic Analyzer            |—                          |Transitive panics |
|22   |Dead Code Analyzer        |—                          |Unused code       |
|23   |Narrowing Checker         |arr[0], bad bounds         |Dynamic index risk|
|24   |Resource Safety Checker   |Unclosed resource          |—                 |
|25   |Concurrency Safety Checker|Data races *(future)*      |—                 |
|26   |`%rust` Isolator          |Bad Rust, ABI mismatch     |`unsafe` usage    |
|27   |Compiler                  |—                          |—                 |
|28   |Bytecode Verifier         |Malformed bytecode         |—                 |
|29   |Bytecode `.nyxb`          |—                          |—                 |
|30   |VM Startup                |Bad signature, OOM         |—                 |
|31   |VM Execution              |—                          |—                 |
|32   |VM Numeric Guard          |→ `err()`                  |—                 |
|33   |VM Memory Guard           |→ `err()`                  |—                 |
|34   |VM Bounds Guard           |→ `err()`                  |—                 |
|35   |VM Unexpected Error Trap  |Structured report          |—                 |

-----

## Project Structure

Nyx is implemented as a Cargo workspace with two crates:

```
nyx/
├── Cargo.toml          # workspace root
├── nyxc/               # Nyx Compiler
│   └── src/
│       ├── main.rs     # CLI entrypoint
│       ├── lexer.rs    # tokenizer
│       ├── parser.rs   # AST builder
│       ├── checks/     # all 26 checker passes
│       └── codegen.rs  # bytecode emitter
└── nyxvm/              # Nyx Virtual Machine
    └── src/
        ├── main.rs     # VM entrypoint
        ├── vm.rs       # bytecode executor
        ├── ownership.rs # ownership tracker
        └── guards/     # runtime safety guards
```

-----

## Quick Reference

### Keywords

|Keyword              |Purpose                               |
|---------------------|--------------------------------------|
|`#fn`                |Top-level function declaration        |
|`#class`             |Top-level class declaration           |
|`#namespace`         |Top-level namespace grouping          |
|`let`                |Variable binding                      |
|`create`             |Class field declaration block         |
|`match`              |Pattern matching                      |
|`if / else if / else`|Conditional                           |
|`while / loop / for` |Loops                                 |
|`break`              |Exit loop with optional value         |
|`return`             |Explicit return                       |
|`await`              |Wait for `%async` or `%spawn` function|
|`panic`              |Unrecoverable error                   |
|`ok / err`           |Result constructors                   |

### Operators

|Operator              |Meaning               |Note                            |
|----------------------|----------------------|--------------------------------|
|`&x`                  |Immutable borrow      |                                |
|`&mut x`              |Mutable borrow        |`x` must be `%mut`              |
|`?`                   |Propagate error       |Only in `%Result`-returning fns |
|`::`                  |Namespace path        |e.g. `math::add`                |
|`1..10`               |Range (inclusive)     |Used in `for` and `match`       |
|`->`                  |Return type annotation|Also used in `%solve`           |
|`+  -  *  /  %`       |Arithmetic            |Spaces required around operators|
|`==  !=  <  >  <=  >=`|Comparison            |                                |
|`&&  ||  !`           |Logical               |                                |