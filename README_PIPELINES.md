# Nyx Compiler Pipeline — Stage Reference

A detailed breakdown of all 35 stages in the Nyx compilation and execution pipeline. Every stage has a single responsibility. Errors are caught as early as possible — ideally at compile time — so that runtime is as safe and predictable as possible.

**Key:**

- ❌ **Hard Error** — compilation halts immediately
- ⚠️ **Warning** — compilation continues, user is informed
- ✅ **Pass** — stage completed successfully, pipeline continues

-----

## Part 1 — Frontend (Source → AST)

-----

### Stage 1 — Lexer

**Responsibility:** Turn raw `.nyx` source text into a flat stream of tokens.

The lexer is the first thing that touches source code. It reads characters one at a time and groups them into meaningful units called tokens. No understanding of grammar or meaning happens here — just recognition.

**What it does:**

- Reads the source file character by character
- Groups characters into tokens: identifiers, literals, operators, punctuation, prefixes
- Recognizes the `%`, `#`, and `!` prefix characters as semantically significant
- Applies whitespace-sensitive operator disambiguation — `a-b` is one identifier, `a - b` is subtraction
- Strips comments (`//`, `/* */`, `///`) from the token stream while preserving doc comment tokens for later
- Tracks line and column numbers on every token for error reporting

**Whitespace disambiguation rule:**

- No spaces around a symbol → part of the identifier (e.g. `my-var`, `!make`, `%suppress-warnings`)
- Spaces on both sides → operator (e.g. `a - b`, `x + y`)
- Space on only one side → ❌ hard error: ambiguous token

**Token types produced:**

```
Ident("x")          // identifier
Directive("mut")    // % prefixed token (% stripped, name kept)
TopLevel            // # prefix marker
Int(42)             // integer literal
Float(3.14)         // float literal
Str("hello")        // string literal
True / False        // %true / %false
Void                // %void
Semi                // ;
LBrace / RBrace     // { }
LParen / RParen     // ( )
LBracket / RBracket // [ ]
Arrow               // ->
ColonColon          // ::
Colon               // :
Eq                  // =
Plus / Minus / Star / Slash / Percent
AmpAmp / PipePipe / Bang
EqEq / BangEq / Lt / Gt / LtEq / GtEq
Amp                 // & (borrow)
AmpMut              // &mut
Question            // ?
DotDot              // ..
Dot                 // .
Comma               // ,
Hash                // # (top-level marker)
DocComment("...")   // /// doc comment
```

**Errors:**

- ❌ Unrecognized character that cannot form any valid token
- ❌ Unterminated string literal
- ❌ Unterminated block comment `/* ...`
- ❌ Ambiguous operator (space on one side only)

**Output:** A flat `Vec<Token>` with source location attached to each token.

-----

### Stage 2 — `%make` Pass

**Responsibility:** Extract and execute compile-time configuration before any other analysis begins.

The `%make` pass runs immediately after lexing, before the full parser. It scans the token stream for the `#fn %make()` block, evaluates it in isolation, and produces a `CompileConfig` struct that all later stages read from.

**What it does:**

- Scans the token stream for `#fn %make() { ... }`
- Validates the structure of `%make` — it must be a function with no parameters and no return type
- Checks for `%logic-%make` before allowing any `if`, `while`, `for`, or `match` inside `%make`
- Evaluates all `let %directive = value;` assignments
- Merges top-level `#import`, `#use`, and `#def` shorthands found anywhere in the token stream into their list equivalents inside the config
- Fires `%when-compile` `%rust` hooks by passing them to the `%rust` isolator for immediate execution
- Produces the final `CompileConfig` that controls all downstream behavior

**`%make` only directives validated here:**

```
%suppress-warnings, %target, %entry, %strict
%hard, %when-run, %when-compile
%import, %use, %def
%repl, %async, %self, %logic-%make
```

**Shorthand merging:**

```nyx
// These at top level...
#import math;
#use math::add;
#def "mylib.rs" as mylib;

// ...are merged into %make as if you had written:
let %import = [math];
let %use = [math::add];
let %def = ["mylib.rs" as mylib];
```

**Errors:**

- ❌ `%make` has parameters or a return type annotation
- ❌ More than one `%make` function in the file
- ❌ Control flow inside `%make` without `%logic-%make = %true`
- ❌ Unknown directive inside `%make`
- ❌ `%when-compile` `%rust` block fails to compile

**Output:** A `CompileConfig` struct consumed by all later stages.

-----

### Stage 3 — Parser

**Responsibility:** Turn the token stream into an Abstract Syntax Tree (AST).

The parser reads the flat token stream produced by the lexer and builds a structured tree that represents the grammatical meaning of the program. This is where Nyx’s syntax rules are enforced.

**What it does:**

- Consumes the token stream left-to-right using a recursive descent parser
- Builds an AST with nodes for: functions, classes, namespaces, let bindings, expressions, blocks, control flow, match arms, etc.
- Tracks scope depth — depth 0 is the Main scope
- Enforces the single-return-value-per-scope rule: scans each block for bare expressions, errors if more than one found
- Enforces `#` prefix at depth 0: `fn`, `class`, `namespace` without `#` at top level → warning
- Hard errors on `#` prefix inside any inner block
- Parses `%nyx(){}` and `%rust(){}` and other language blocks as expression nodes
- Parses `%solve` declarations as a special solver AST node
- Parses `create { }` blocks inside classes as field declaration nodes
- Parses mass `let = { ... }` as a multi-binding node

**Scope depth tracking:**

```
depth 0  →  Main scope  →  # prefix required, warns if missing
depth 1  →  fn/class body  →  # prefix is hard error
depth 2+ →  nested blocks  →  # prefix is hard error
```

**Errors:**

- ❌ Two bare expressions in the same scope
- ❌ `#` prefix used inside an inner block
- ❌ Malformed expression (missing operand, unclosed bracket, etc.)
- ❌ `create` block outside a `class`
- ❌ `%solve` used inside a non-`%Result`-returning scope
- ⚠️ Top-level declaration missing `#` prefix

**Output:** A fully structured AST ready for semantic analysis.

-----

## Part 2 — Validation (AST → Checked AST)

-----

### Stage 4 — Directive Validator

**Responsibility:** Verify every `%`-prefixed token is either a known directive or a legitimate identifier.

After parsing, the directive validator walks the AST and inspects every node that was parsed from a `%`-prefixed token. It cross-references against the hardcoded directive list.

**What it does:**

- Walks every node in the AST that contains a `%`-prefixed name
- Checks the name against the complete hardcoded directive list
- For unknown `%` names: emits a warning with a suggestion (using edit-distance to find the closest known directive)
- For `%make`-only directives found outside `%make`: hard error
- For type directives used in the wrong position (e.g. `%mut` as a return type): hard error
- Respects `%suppress-warnings` from the `CompileConfig`

**Example suggestions:**

```
%mutable  →  ⚠️ Unknown directive. Did you mean %mut?
%pubic    →  ⚠️ Unknown directive. Did you mean %pub?
%repl     →  ❌ %repl is only valid inside %make
```

**Errors:**

- ❌ `%make`-only directive used outside `%make`
- ❌ Type directive used in a position that requires a value directive (e.g. `let %i32 %mut x` — wrong order)
- ⚠️ Unrecognized `%` identifier (not in directive list)

**Output:** AST with all directive nodes validated and annotated.

-----

### Stage 5 — Import Cycle Detector

**Responsibility:** Detect circular dependencies in the `%import` graph before they can poison type inference.

Import cycles cause infinite loops in any compiler that resolves imports lazily. Nyx detects them up front, before any type information is resolved.

**What it does:**

- Reads the `%import` list from `CompileConfig`
- Recursively loads the `%make` pass output from each imported module to discover their own `%import` lists
- Builds a directed import graph
- Runs a depth-first search for cycles (standard DFS with a visited + in-stack set)
- If a cycle is found, reports the full chain

**Example error:**

```
❌ Import cycle detected:
   main.nyx → math.nyx → utils.nyx → main.nyx
```

**Errors:**

- ❌ Any cycle in the import graph, however long

**Output:** A validated, cycle-free import graph. All imported module ASTs are loaded and ready.

-----

### Stage 6 — Type Inference

**Responsibility:** Walk the AST and assign a type to every expression and binding that doesn’t have one explicitly.

Nyx is statically typed but inference is pervasive — you rarely need to annotate types. This stage fills in all the blanks.

**What it does:**

- Walks the AST bottom-up, propagating types from leaves to roots
- Infers variable types from their initializer expressions (`let x = 42` → `x: %i64`)
- Infers function return types from bare expressions in their bodies
- Resolves generic type parameters `<T>` at call sites by unifying with the argument types
- Propagates types through `if/else` branches (both branches must agree)
- Propagates types through `match` arms (all arms must agree)
- Infers the type of `%nyx(){}` blocks from their bare return expression
- Infers `%Result<T>` inner type from `ok(value)` / `err(msg)` calls
- Infers array element types from literals (`[1, 2, 3]` → `[%i64]`)

**Default numeric types:**

- Integer literals without annotation → `%i64`
- Float literals without annotation → `%f64`

**Errors:**

- ❌ Type cannot be inferred (no initializer, no annotation, no context)
- ❌ Branches of `if/else` return conflicting types
- ❌ Arms of `match` return conflicting types
- ❌ Generic `<T>` cannot be resolved from call site arguments

**Output:** AST with every node annotated with a resolved type.

-----

### Stage 7 — Type Checker

**Responsibility:** Verify all type annotations and usages are consistent and valid across the program.

Type inference fills in types. The type checker verifies they are correct and consistent.

**What it does:**

- Verifies every function call passes arguments of the correct types
- Verifies every assignment has a compatible right-hand side type
- Verifies return expressions match the declared return type
- Validates `std::convert(target, value)` — checks that source and target are in the same type family
- Hard errors on cross-family conversion without `%hard = [std::convert]`
- Validates that `%Result<T>` is only used as a return type, not as a plain variable type directly
- Validates that generics `<T>` are used consistently — if `T` is inferred as `%i32` at one call site, it must be `%i32` at all call sites in the same monomorphized instance
- Validates `%bool` contexts — conditions in `if`, `while`, `loop` must be boolean or falsy/truthy

**Type families for `std::convert`:**

```
Integer family:  %i8, %i16, %i32, %i64, %u8, %u16, %u32, %u64
Float family:    %f32, %f64
Text family:     %str, %char
Boolean family:  %bool
```

**Errors:**

- ❌ Type mismatch at function call site
- ❌ Type mismatch in assignment
- ❌ Return type doesn’t match declared signature
- ❌ `std::convert` used across type families without `%hard`
- ❌ Any implicit coercion — Nyx has zero implicit coercion

**Output:** Fully type-checked AST.

-----

### Stage 8 — Strict Inference Validator

**Responsibility:** A second pass ensuring no type variable was left unresolved or ambiguous by inference.

Type inference can sometimes produce multiple valid candidates for a type. This stage ensures every single type in the program is unambiguous.

**What it does:**

- Walks every type annotation in the AST
- Checks for any type variable that was not fully resolved to a concrete type
- If a type is still ambiguous (multiple valid inferences), forces the user to add an explicit annotation
- Checks that generic parameters have been fully monomorphized — no open `<T>` remaining

**Example:**

```nyx
let x = [];     // ❌ type of array elements is unknown — annotate: let %i32[] x = [];
```

**Errors:**

- ❌ Any unresolved or ambiguous type variable remaining after inference
- ❌ Any open generic parameter that was never resolved

**Output:** AST guaranteed to have a concrete, unambiguous type on every single node.

-----

### Stage 9 — Numeric Safety Checker

**Responsibility:** Catch numeric errors — overflow, underflow, division by zero — at compile time wherever statically possible.

**What it does:**

- Evaluates constant numeric expressions at compile time
- Checks integer literals against their annotated type’s range (e.g. `let %i8 x = 200` → max is 127)
- Detects obvious division by zero in constant expressions (`x / 0`)
- Detects obvious modulo by zero (`x % 0`)
- Tracks numeric value ranges through branches where the range can be statically determined
- Warns on arithmetic that could overflow at runtime (non-constant operands)
- Warns on narrowing conversions via `std::convert` (e.g. `%i64` → `%i8`)
- Emits `OVERFLOW_GUARD` bytecode annotations for non-static arithmetic, consumed by Stage 32

**Type ranges:**

```
%i8:   -128 to 127
%i16:  -32768 to 32767
%i32:  -2147483648 to 2147483647
%i64:  -9223372036854775808 to 9223372036854775807
%u8:   0 to 255
%u16:  0 to 65535
%u32:  0 to 4294967295
%u64:  0 to 18446744073709551615
```

**Errors:**

- ❌ Compile-time integer literal out of range for annotated type
- ❌ Constant division by zero
- ❌ Constant modulo by zero
- ⚠️ Non-constant arithmetic that could overflow at runtime
- ⚠️ Narrowing `std::convert` that could lose precision

**Output:** AST with overflow guards annotated on all non-constant numeric operations.

-----

### Stage 10 — Ownership Checker

**Responsibility:** Validate the ownership model — every value is owned by exactly one binding at all times.

**What it does:**

- Tracks the ownership state of every binding in every scope: `Owned`, `Moved`, or `Dropped`
- When a value is assigned to a new binding (`let y = x`), marks `x` as `Moved`
- Hard errors on any read of a `Moved` binding
- Tracks ownership transfer through function calls — if a function takes ownership of an argument, the caller loses it
- Determines the correct drop point for every value (end of its owning scope)
- Validates that every owned value is dropped exactly once
- Checks drop order is deterministic: reverse of declaration order within a scope

**Ownership states:**

```
Owned   →  binding holds a live value
Moved   →  value was transferred, binding is a tombstone
Dropped →  value was freed at end of scope
```

**Errors:**

- ❌ Read of a `Moved` binding (use-after-move)
- ❌ Any path that could result in a value being dropped more than once
- ❌ Any path that could result in a value never being dropped (leak)

**Output:** AST annotated with ownership states and drop points.

-----

### Stage 11 — Borrow Checker

**Responsibility:** Enforce the borrow rules — no conflicting borrows may coexist.

**What it does:**

- Tracks all active borrows at every point in the program
- Enforces: at most one `&mut` borrow active at any time
- Enforces: `&` and `&mut` borrows cannot coexist
- Allows: multiple `&` borrows simultaneously
- Validates borrows across function call boundaries — if a function takes `&x`, the borrow must be released before `x` is moved or mutated
- Tracks borrow scopes precisely — a borrow begins at `&x` and ends at the last use of the reference

**Borrow rules:**

```
&x active, &x again    →  ✅ allowed (multiple immutable)
&mut x active, &x      →  ❌ conflict
&x active, &mut x      →  ❌ conflict
&mut x active, &mut x  →  ❌ conflict
```

**Errors:**

- ❌ Mutable and immutable borrow active simultaneously
- ❌ Two mutable borrows active simultaneously
- ❌ Borrow active when the owner is moved or dropped

**Output:** AST with borrow scopes validated and annotated.

-----

### Stage 12 — Lifetime Checker

**Responsibility:** Ensure no reference ever outlives the value it points to.

**What it does:**

- Assigns a lifetime to every reference (`&x`, `&mut x`) based on the scope of its owner
- Validates that references are never returned from a function if they point to a local variable (the local would be dropped, leaving a dangling reference)
- Validates that references stored in data structures don’t outlive the data they point to
- Works in tandem with the borrow checker — borrows must end before their owner’s lifetime ends

**Example:**

```nyx
#fn bad() -> &%i32 {
    let x = 5;
    &x          // ❌ x is dropped at end of fn, returning a dangling reference
};
```

**Errors:**

- ❌ Returning a reference to a local variable
- ❌ Reference stored in a longer-lived location that outlives its owner
- ❌ Any reference that could point to a dropped value

**Output:** AST with lifetime annotations validated.

-----

### Stage 13 — Drop Order Validator

**Responsibility:** Verify that the order in which values are dropped is deterministic and correct on every possible code path.

**What it does:**

- Walks every scope in the AST
- Verifies values are dropped in reverse declaration order (last declared, first dropped)
- Validates drops across all branching paths — `if/else`, `match`, early `return`, `?` propagation
- Ensures a borrow cannot outlive the drop of its owner on any code path
- Validates that `%spawn` threads don’t hold borrows that could outlive the spawning scope

**Drop order rule:**

```nyx
{
    let a = ...;   // dropped 3rd
    let b = ...;   // dropped 2nd
    let c = ...;   // dropped 1st
};                 // drops: c, then b, then a
```

**Errors:**

- ❌ Any code path where drop order is non-deterministic
- ❌ Borrow that outlives its owner’s drop on any branch

**Output:** AST with drop order confirmed correct on all paths.

-----

### Stage 14 — Double Free Detector

**Responsibility:** Dedicated pass to guarantee no value is ever freed twice under any execution path.

**What it does:**

- Constructs a control flow graph (CFG) of the entire program
- Walks every path through the CFG tracking drop events
- Verifies no single value has more than one drop event on any path
- Checks that move operations never leave a binding that could be dropped separately
- Validates that `%def` Rust functions cannot trigger a double free by returning an already-owned value

**Errors:**

- ❌ Any execution path where the same value is dropped more than once

**Output:** Verified CFG with no double-free paths.

-----

### Stage 15 — Mutability Checker

**Responsibility:** Ensure immutability is airtight — nothing that isn’t declared `%mut` is ever mutated.

**What it does:**

- Tracks the mutability of every binding
- Hard errors on any assignment to an immutable binding after initialization
- Hard errors on mutation through an immutable borrow (`&x` cannot be used to write)
- Validates that `&mut x` is only created from a `%mut` binding
- Validates that class methods that mutate `%self` take `&mut %self`, not `&%self`

**Errors:**

- ❌ Assignment to an immutable binding
- ❌ Mutation through an immutable borrow
- ❌ `&mut` borrow of a non-`%mut` binding
- ❌ Mutating method called on an immutable binding

**Output:** AST with mutability validated throughout.

-----

### Stage 16 — Initialization Checker

**Responsibility:** Ensure every variable is initialized before it is read, on every possible code path.

**What it does:**

- Tracks the initialization state of every binding: `Uninitialized`, `MaybeInitialized`, or `Initialized`
- Follows all branches — if only one branch of an `if/else` initializes `x`, then after the branch `x` is `MaybeInitialized`
- Hard errors on reading a `MaybeInitialized` or `Uninitialized` binding
- Validates function parameters are passed at every call site (no optional parameters without defaults)

**States:**

```
Uninitialized     →  let x: %i32;  (declared but not set)
MaybeInitialized  →  only some branches initialize it
Initialized       →  definitely has a value
```

**Errors:**

- ❌ Reading an `Uninitialized` binding
- ❌ Reading a `MaybeInitialized` binding

**Output:** AST with initialization states confirmed on all paths.

-----

### Stage 17 — Exhaustiveness Checker

**Responsibility:** Validate that `match` statements cover every possible case and that all `%Result<T>` values are handled.

**What it does:**

- Walks every `match` expression in the AST
- Builds the set of possible values for the matched expression based on its type
- Verifies the union of all match arms covers the complete set
- Suggests specific missing arms in the error message
- Walks every `%Result<T>` value in the program to confirm it is matched, `?`’d, or handled

**Exhaustiveness for common types:**

```
%bool  →  must cover %true and %false (or _)
%i32   →  must have a _ catch-all
enum   →  must cover every variant (or _)
```

**Errors:**

- ❌ `match` missing one or more arms (with suggestions)
- ❌ `%Result<T>` value that is never handled

**Output:** AST with all match expressions confirmed exhaustive.

-----

### Stage 18 — Forced Error Handling Validator

**Responsibility:** A dedicated pass ensuring no `%Result<T>` is ever silently discarded.

**What it does:**

- Walks every function call in the AST
- For every call to a function that returns `%Result<T>`, verifies the return value is used
- “Used” means: assigned to a binding, matched, passed to another function, or `?`’d
- Validates that `panic` is not used in places where `%Result` is clearly the right tool

**Errors:**

- ❌ Calling a `%Result`-returning function and discarding the return value entirely
- ❌ Calling `panic` where the error is clearly recoverable (e.g. inside a `%Result`-returning function)

**Output:** AST with all error handling confirmed present.

-----

### Stage 19 — Reachability Checker

**Responsibility:** Detect code that can never execute, and enforce the single-return-value rule at the control flow level.

**What it does:**

- Walks the CFG looking for nodes with no incoming edges (unreachable code)
- Hard errors on any scope containing two bare expressions — enforces the single return value rule at a deeper level than the parser
- Warns on statements after a bare return expression that can never execute
- Warns on `loop` bodies after a `break` that covers all paths

**Errors:**

- ❌ Two bare expressions in the same scope (deeper check than Stage 3)
- ⚠️ Code after a return value that can never be reached

**Output:** CFG with unreachable nodes flagged.

-----

### Stage 20 — Control Flow Analyzer

**Responsibility:** Ensure every possible execution path through every function ends with a return value of the correct type.

**What it does:**

- Walks the CFG of every function
- Verifies that every path from the entry node to an exit node passes through a return or bare expression of the declared return type
- Validates that `loop` constructs either have a reachable `break value` on every path, or never exit
- Validates that `?` is only used inside `%Result`-returning functions
- Validates that `break` values inside `loop` are all of the same type

**Errors:**

- ❌ Function that might not return on some branch (missing return on a path)
- ❌ `loop` with no reachable `break` that isn’t declared as diverging
- ❌ `?` used inside a function that doesn’t return `%Result<T>`
- ❌ `break` values of inconsistent types within the same `loop`

**Output:** CFG with all control flow paths verified to terminate correctly.

-----

### Stage 21 — Panic Analyzer

**Responsibility:** Make panics visible throughout the call graph so programmers know which functions can fail catastrophically.

**What it does:**

- Builds the full call graph of the program
- Marks every function that directly calls `panic()` as “can panic”
- Propagates the “can panic” mark transitively — if `foo` calls `bar` and `bar` can panic, then `foo` can panic
- Warns on every function marked “can panic”
- In `%strict` mode: functions that can panic must be explicitly annotated (future: `#fn %may-panic`)

**Errors:**

- ⚠️ Function that can transitively reach a `panic()` call
- ⚠️ Suggestion to wrap in `%Result` if the panic is clearly recoverable

**Output:** Call graph with panic propagation annotated.

-----

### Stage 22 — Dead Code Analyzer

**Responsibility:** Warn on code that exists but is never used.

**What it does:**

- Tracks every function, class, namespace, variable, `%use`’d item, and `%def`’d library
- Marks each as “used” when it appears in a reachable expression
- Warns on anything never marked as used
- Respects `%suppress-warnings` from `CompileConfig`
- Does NOT warn on `%pub` items — they may be used by external importers

**Warnings:**

- ⚠️ Unused variable
- ⚠️ Unused function (non-`%pub`)
- ⚠️ Unused class (non-`%pub`)
- ⚠️ `%use`’d item never called
- ⚠️ `%def`’d library never called

**Output:** AST with dead code annotated (for warnings only, no hard errors).

-----

### Stage 23 — Narrowing Checker

**Responsibility:** Validate array access safety at compile time — especially Nyx’s 1-indexed guarantee.

**What it does:**

- Walks every array index expression in the AST
- Hard errors on any constant index of `0` — arrays are 1-indexed
- Hard errors on any constant index known to be out of bounds (e.g. indexing a 3-element array at `[5]`)
- Warns on dynamic (runtime-computed) indices that could potentially be 0 or out of bounds
- Emits `BOUNDS_GUARD` bytecode annotations for dynamic accesses, consumed by Stage 34

**Errors:**

- ❌ `arr[0]` — zero index on any array
- ❌ `arr[n]` where `n` is a constant greater than the array’s known length
- ⚠️ Dynamic index with no statically-provable lower bound of 1

**Output:** AST with bounds guards annotated on all dynamic array accesses.

-----

### Stage 24 — Resource Safety Checker

**Responsibility:** Ensure all resources (files, handles, connections, etc.) are always closed on every code path.

**What it does:**

- Identifies all values of resource types (types that require explicit cleanup — file handles, network connections, etc.)
- Tracks resource state: `Open` or `Closed`
- Verifies every resource is in `Closed` state at the end of its owning scope
- Validates resources are closed on all paths including early `return`, `?` propagation, and `panic`
- Validates resources are closed even in error branches — no leak on failure

**Resource types are identified by:**

- Implementing a `drop` handler (future: via a `%resource` annotation on the class)
- Being returned by known resource-creating standard library functions

**Errors:**

- ❌ Resource-owning value goes out of scope without being closed
- ❌ Early return or `?` propagation that skips resource cleanup

**Output:** AST with resource lifetimes validated.

-----

### Stage 25 — Concurrency Safety Checker

**Responsibility:** Validate thread safety when green threads are in use.

> **Status:** Reserved. This stage activates automatically when `%async = %true` is set in `%make` and `#fn %spawn` functions are present.

**What it does (when active):**

- Validates that all values moved into a `%spawn` function are fully owned — not borrowed
- Hard errors on capturing a reference inside a `%spawn` function (references cannot cross thread boundaries)
- Validates that `await` is only used inside `%async` or `%spawn` functions
- Ensures ownership is returned correctly via the function’s return value
- Validates no shared mutable state exists between threads (ownership transfer is the only allowed communication)

**Errors:**

- ❌ Moving a borrowed value into a `%spawn` function
- ❌ Capturing a reference inside a `%spawn` function
- ❌ `await` used outside an `%async` or `%spawn` function
- ❌ Shared mutable state between threads

**Output:** AST with thread safety validated.

-----

### Stage 26 — `%rust` Isolator

**Responsibility:** Sandbox all Rust interop — `%rust(){}` blocks, `#rust {}` expressions, and `%def` linkages — and verify they are safe to call from Nyx.

This is the security boundary between Nyx’s safe world and raw Rust.

**What it does:**

- Collects every `%rust(){}` block, `#rust {}` expression, `%when-run` hook, and `%when-compile` hook from the AST
- For each `.rs` file in `%def`: reads it and extracts all `#[nyx_abi]`-annotated functions
- For each `.dll` / `.so` in `%def`: reads the exported symbol table
- Runs `rustc` on every `.rs` source in isolation — hard errors if compilation fails
- Maps Rust types to Nyx types using the hardcoded type table — hard errors on unmappable types
- Generates Nyx stub function nodes for each `#[nyx_abi]` function and injects them into the AST
- Validates ABI compatibility for `.dll` / `.so` files
- Audits every `%rust` block for `unsafe` Rust — warns on every `unsafe` keyword found
- Wraps all `%rust` return values in `%Result` — Rust panics are caught and converted to `err()`

**Type mapping:**

```
Rust &str / String        →  Nyx %str
Rust i8/i16/i32/i64       →  Nyx %i8/%i16/%i32/%i64
Rust u8/u16/u32/u64       →  Nyx %u8/%u16/%u32/%u64
Rust f32/f64              →  Nyx %f32/%f64
Rust bool                 →  Nyx %bool
Rust ()                   →  Nyx %void
Rust Result<T, String>    →  Nyx %Result<T>
Anything else             →  ❌ hard error — cannot cross boundary
```

**Errors:**

- ❌ `.rs` file fails to compile with `rustc`
- ❌ Rust function uses a type that has no Nyx mapping
- ❌ `.dll` / `.so` ABI is incompatible
- ❌ `%when-compile` `%rust` block fails to compile or run
- ⚠️ `unsafe` Rust found in any `%rust` block or `%def` file

**Output:** AST with Rust stub functions injected and all interop validated.

-----

## Part 3 — Code Generation

-----

### Stage 27 — Compiler

**Responsibility:** Translate the fully checked and annotated AST into Nyx bytecode.

This is the code generation stage. By the time execution reaches here, the AST is guaranteed correct by all 26 preceding passes.

**What it does:**

- Walks the AST in execution order
- Assigns a register to every value (register-based VM)
- Emits bytecode instructions for every AST node
- Inlines every `%use`’d and `%import`’d function at its call sites — no dynamic dispatch
- Emits `DROP` instructions at the end of every scope for owned values, in reverse declaration order
- Emits `BORROW` and `RELEASE` instructions around borrow scopes
- Bakes the ownership table (register → owner mapping) into the bytecode header
- Inserts `OVERFLOW_GUARD`, `BOUNDS_GUARD`, and `OOM_GUARD` instructions where annotated by earlier passes
- Emits `FFI_CALL` instructions for `%def` function calls
- Handles `%solve` by emitting calls to the built-in solver runtime

**Core instruction set:**

```
MOV   reg, value       — move value into register
COPY  reg, reg         — copy value (for types that impl copy)
DROP  reg              — drop owned value, free memory
BORROW reg, reg        — create immutable borrow
BORROW_MUT reg, reg    — create mutable borrow
RELEASE reg            — release borrow
CALL  fn, [args]       — call function
RET   reg              — return value
JMP   label            — unconditional jump
JMP_IF reg, label      — conditional jump
JMP_NOT reg, label     — conditional jump (false)
ADD / SUB / MUL / DIV / MOD
EQ / NEQ / LT / GT / LTE / GTE
AND / OR / NOT
ARRAY_NEW  reg, size   — allocate array
ARRAY_GET  reg, reg, reg — indexed read
ARRAY_SET  reg, reg, reg — indexed write
OVERFLOW_GUARD reg     — check for overflow before arithmetic
BOUNDS_GUARD reg, reg  — check index against array length
OOM_GUARD  size        — check available memory before allocation
FFI_CALL  sym, [args]  — call into Rust via ABI boundary
SOLVE     reg, expr    — invoke equation solver
SPAWN     fn, [args]   — spawn green thread
AWAIT     reg          — wait for spawned thread
PANIC     msg          — unrecoverable error
```

**Output:** Raw bytecode buffer + ownership table + function table.

-----

### Stage 28 — Bytecode Verifier

**Responsibility:** Final sanity check on the emitted bytecode before it is ever handed to the VM.

This is the last gate before execution. It exists as a defense-in-depth measure against compiler bugs.

**What it does:**

- Reads the raw bytecode buffer linearly
- Verifies every jump target lands on a valid instruction boundary
- Verifies no register is read before it is written (no use-before-def in bytecode)
- Verifies the ownership table is internally consistent — every register referenced in the table exists in the bytecode
- Verifies all `OVERFLOW_GUARD`, `BOUNDS_GUARD`, and `OOM_GUARD` instructions are present where the AST passes annotated them
- Verifies `DROP` instructions are present for every owned register at its annotated drop point
- Verifies `BORROW` / `RELEASE` pairs are balanced
- Verifies `FFI_CALL` targets exist in the loaded symbol table

**Errors:**

- ❌ Jump to an address outside the bytecode buffer
- ❌ Jump to the middle of an instruction (misaligned target)
- ❌ Register read before write
- ❌ Ownership table references a nonexistent register
- ❌ Missing guard instruction
- ❌ Missing `DROP` instruction
- ❌ Unbalanced `BORROW` / `RELEASE`
- ❌ `FFI_CALL` to an unknown symbol

**Output:** Validated, ready-to-execute `.nyxb` bytecode file.

-----

### Stage 29 — Bytecode (`.nyxb`)

**Responsibility:** The final artifact of compilation — a flat binary file ready to hand to NyxVM.

**File format:**

```
[4 bytes]   Magic number: 0x4E595842 ("NYXB")
[4 bytes]   Version: major.minor.patch
[4 bytes]   Flags (debug/release, async enabled, etc.)
[8 bytes]   Ownership table offset
[8 bytes]   Function table offset
[8 bytes]   Symbol table offset (FFI)
[8 bytes]   Bytecode section offset
[N bytes]   Ownership table
[N bytes]   Function table (name → bytecode address)
[N bytes]   Symbol table (FFI bindings)
[N bytes]   Bytecode instructions
```

**Output:** A `.nyxb` file on disk.

-----

## Part 4 — Runtime (NyxVM)

-----

### Stage 30 — VM Startup

**Responsibility:** Initialize the VM and validate the bytecode before executing a single instruction.

**What it does:**

- Reads the `.nyxb` file header
- Validates the magic number (`0x4E595842`) — hard error if wrong file
- Validates the bytecode version against the VM version — hard error if incompatible
- Loads the ownership table, function table, and symbol table into memory
- Initializes the register file (all registers start as `Uninitialized`)
- Initializes the ownership tracker (maps registers to their owner scopes)
- Performs initial heap allocation with OOM detection — if the system cannot provide the minimum required memory, exits with a structured error
- Loads and links all `%def` native libraries
- Fires `%when-run` `%rust` hooks
- Sets the instruction pointer to the entry function

**Errors:**

- ❌ Bad magic number (not a `.nyxb` file)
- ❌ Version mismatch between bytecode and VM
- ❌ OOM on startup allocation
- ❌ Failed to load a `%def` native library

**Output:** VM fully initialized, ready to execute.

-----

### Stage 31 — VM Execution

**Responsibility:** Execute bytecode instructions one at a time, maintaining the full program state.

**What it does:**

- Runs the fetch-decode-execute loop on the bytecode
- Maintains the register file — reads and writes register values per instruction
- Maintains the ownership tracker — updates ownership state on `MOV`, `DROP`, `BORROW`, `RELEASE`
- Enforces ownership rules at runtime as a final safety net (defense in depth — the compiler should have caught everything, but the VM double-checks)
- Handles `%Result` propagation — `?` instructions check `result[1]` and unwind if `%false`
- Handles `panic` by unwinding the stack and transitioning to the Unexpected Error Trap (Stage 35)
- Manages deterministic drops at scope boundaries — executes `DROP` instructions in the emitted order
- Tracks open resource handles and forces close on scope exit
- Dispatches `SPAWN` instructions to the green thread scheduler
- Dispatches `AWAIT` instructions to block on a scheduled thread’s completion
- Dispatches `FFI_CALL` instructions to the native library ABI boundary
- Dispatches `SOLVE` instructions to the built-in equation solver runtime

**Output:** Running program. Delegates to Stages 32–35 for safety guard handling.

-----

### Stage 32 — VM Numeric Runtime Guard

**Responsibility:** Catch numeric errors at runtime that could not be resolved statically at compile time.

**What it does:**

- Executes before every `ADD`, `SUB`, `MUL`, `DIV`, `MOD` instruction where an `OVERFLOW_GUARD` was emitted
- Checks the operands and result for overflow/underflow relative to the result register’s type
- Checks the divisor for zero before every `DIV` and `MOD`
- In `%target = "debug"`: traps immediately with full source location in the error message
- In `%target = "release"`: converts the error into a `%Result` `err()` and unwinds gracefully — no crash

**Errors → converted to `err()`:**

- Integer overflow
- Integer underflow
- Division by zero → `err("division by zero at line N")`
- Modulo by zero → `err("modulo by zero at line N")`

**Output:** Either continues execution normally, or returns a structured `err()` and unwinds.

-----

### Stage 33 — VM Memory Runtime Guard

**Responsibility:** Monitor memory safety at runtime — OOM, stack overflow, and ownership integrity.

**What it does:**

- Before every heap allocation (`ARRAY_NEW`, class construction, etc.): checks available memory. OOM → `err()`, never a crash
- Maintains a stack depth counter incremented on every `CALL` and decremented on every `RET`
- If stack depth exceeds a configurable limit: stack overflow → `err("stack overflow")`, not a segfault
- Before every `DROP` instruction: validates the target register is in `Owned` state — double-free attempt → `err()` and report

**Errors → converted to `err()` or structured report:**

- OOM → `err("out of memory")`
- Stack overflow → `err("stack overflow")`
- Double-free attempt → structured error report (Stage 35)

**Output:** Either continues execution, or returns structured `err()` and unwinds.

-----

### Stage 34 — VM Bounds Runtime Guard

**Responsibility:** Validate all array accesses at runtime against the actual array length.

**What it does:**

- Executes before every `ARRAY_GET` and `ARRAY_SET` instruction where a `BOUNDS_GUARD` was emitted
- Checks that the index is ≥ 1 (Nyx is 1-indexed — 0 should have been caught at compile time, but the VM checks anyway)
- Checks that the index is ≤ the array’s current length
- Out-of-bounds access → `err()` with the exact index and array length reported

**Errors → converted to `err()`:**

- Index 0 → `err("index 0 is invalid — Nyx arrays are 1-indexed")`
- Index > length → `err("index N out of bounds — array length is M")`

**Output:** Either continues execution, or returns structured `err()` and unwinds.

-----

### Stage 35 — VM Unexpected Error Trap

**Responsibility:** The absolute last line of defense. Catch any error that was not handled by Stages 32–34 and produce a structured, readable report. The program never crashes silently.

**What it does:**

- Wraps the entire VM execution loop in a top-level error handler
- Catches any unhandled VM state, illegal instruction, or internal invariant violation
- Produces a full structured crash report written to stderr:
  
  ```
  ═══════════════════════════════════════════════
  NYX RUNTIME ERROR — Unexpected VM State
  ═══════════════════════════════════════════════
  Source:       main.nyx, line 42
  Instruction:  ARRAY_GET (addr 0x00FF)
  Message:      <description of what went wrong>
  
  Register State:
    r0  =  42        (owned by main)
    r1  =  [1,2,3]   (owned by main)
    r2  =  <moved>
    ...
  
  Ownership Table:
    r0  →  main::x
    r1  →  main::arr
    ...
  
  Call Stack:
    main (main.nyx:42)
    ← process (utils.nyx:17)
    ← entry
  ═══════════════════════════════════════════════
  ```
- Always exits with a non-zero exit code
- Never produces a raw segfault, panic dump, or OS-level crash
- If the crash report itself fails to write (e.g. stderr is closed), falls back to a minimal single-line message and exits

**This stage handles:**

- Any VM internal invariant violation not caught by Stages 32–34
- `panic()` calls that were not caught by an error handler
- Any internal compiler bug that produced invalid bytecode that slipped past Stage 28
- Any OS-level signal (SIGSEGV, SIGFPE, etc.) — trapped and converted to a structured report

**Output:** A structured error report on stderr and a clean non-zero exit. Never a raw crash.

-----

## Summary

|Stage|Name                      |Phase     |Hard Errors                |Warnings          |
|-----|--------------------------|----------|---------------------------|------------------|
|1    |Lexer                     |Frontend  |Malformed tokens           |—                 |
|2    |`%make` Pass              |Frontend  |Bad directives             |—                 |
|3    |Parser                    |Frontend  |Scope violations, bad AST  |Missing `#`       |
|4    |Directive Validator       |Validation|Wrong context              |Unknown `%`       |
|5    |Import Cycle Detector     |Validation|Cycles                     |—                 |
|6    |Type Inference            |Validation|Ambiguous types            |—                 |
|7    |Type Checker              |Validation|Type mismatches            |—                 |
|8    |Strict Inference Validator|Validation|Unresolved types           |—                 |
|9    |Numeric Safety Checker    |Validation|Overflow, div/0            |Runtime risk      |
|10   |Ownership Checker         |Validation|Use-after-move, double free|—                 |
|11   |Borrow Checker            |Validation|Borrow conflicts           |—                 |
|12   |Lifetime Checker          |Validation|Dangling references        |—                 |
|13   |Drop Order Validator      |Validation|Bad drop order             |—                 |
|14   |Double Free Detector      |Validation|Double drop                |—                 |
|15   |Mutability Checker        |Validation|Illegal mutation           |—                 |
|16   |Initialization Checker    |Validation|Uninitialized read         |—                 |
|17   |Exhaustiveness Checker    |Validation|Incomplete match           |—                 |
|18   |Forced Error Handling     |Validation|Ignored Result             |—                 |
|19   |Reachability Checker      |Validation|Two bare expressions       |Unreachable code  |
|20   |Control Flow Analyzer     |Validation|Missing return path        |—                 |
|21   |Panic Analyzer            |Validation|—                          |Transitive panics |
|22   |Dead Code Analyzer        |Validation|—                          |Unused code       |
|23   |Narrowing Checker         |Validation|arr[0], bad bounds         |Dynamic index risk|
|24   |Resource Safety Checker   |Validation|Unclosed resource          |—                 |
|25   |Concurrency Safety Checker|Validation|Data races *(future)*      |—                 |
|26   |`%rust` Isolator          |Validation|Bad Rust, ABI mismatch     |`unsafe` usage    |
|27   |Compiler                  |Code Gen  |—                          |—                 |
|28   |Bytecode Verifier         |Code Gen  |Malformed bytecode         |—                 |
|29   |Bytecode `.nyxb`          |Code Gen  |—                          |—                 |
|30   |VM Startup                |Runtime   |Bad signature, OOM         |—                 |
|31   |VM Execution              |Runtime   |—                          |—                 |
|32   |VM Numeric Guard          |Runtime   |→ `err()`                  |—                 |
|33   |VM Memory Guard           |Runtime   |→ `err()`                  |—                 |
|34   |VM Bounds Guard           |Runtime   |→ `err()`                  |—                 |
|35   |VM Unexpected Error Trap  |Runtime   |Structured report          |—                 |