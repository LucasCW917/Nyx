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
|35   |VM Unexpected Error Trap  |Structured report          |—                 |