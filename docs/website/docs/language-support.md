---
id: language-support
title: Language Support
sidebar_position: 6
---

# Language support

ctx parses source with tree-sitter (Solidity with `solang-parser`). This page lists what ctx extracts from each language and which relationships it records. To index a language without a built-in parser, register a stdio language server; see [Languages beyond the built-in set (LSP)](#languages-beyond-the-built-in-set-lsp).

## Supported languages

| Language | Extensions | Symbols | Edges | Status |
|----------|-----------|---------|-------|--------|
| Rust | `.rs` | Full | Calls, Implements, Uses | Full |
| TypeScript | `.ts` | Full | Calls, Extends, Implements | Full |
| TSX | `.tsx` | Full | Calls, Extends, Implements | Full |
| JavaScript | `.js`, `.mjs`, `.cjs` | Full | Calls, Extends | Full |
| JSX | `.jsx` | Full | Calls, Extends | Full |
| Python | `.py`, `.pyi` | Full | Calls, Extends | Full |
| Go | `.go` | Full | Calls | Full |
| C | `.c`, `.h` when selected as C | Full | Calls | Full |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hh`, `.hpp`, `.hxx`, `.ipp`, `.tpp`, `.h` when selected as C++ | Full | Calls | Full |
| Zig | `.zig` | Full | Calls | Full |
| Solidity | `.sol` | Full | Calls, Uses | Full |
| YAML | `.yaml`, `.yml` | File tracking only | None | Partial |

Every parsed language also records its imports (includes for C and C++) per file in module metadata (the `modules` table), not as edges. Architecture checks read them from there.

## Languages beyond the built-in set (LSP)

To index a language with a stdio language server, such as Kotlin, Scala or Ruby, register the server under `[lsp.<language>]` in `.ctx/config.toml`. ctx then extracts symbols with `textDocument/documentSymbol`, call edges with the call-hierarchy requests, and cross-file references with `textDocument/definition`:

```toml
[lsp.kotlin]
command = "kotlin-language-server"
extensions = ["kt", "kts"]
backend = "lsp"
```

For languages in the community registry, `ctx lsp add <language>` shows what it would install and then writes this block. Server failures never fail indexing: a missing or crashed server prints a warning on stderr, and ctx falls back to tree-sitter (built-in languages) or file-only records (other languages).

See [Add a language via LSP](lsp-languages.md) for the full key reference, and [`ctx lsp`](commands/lsp.md) for the registry commands.

### Hybrid mode for built-in languages

You can pair a built-in language with a language server using `backend = "hybrid"` (the default for configured blocks). tree-sitter still extracts symbols and edges offline. Afterwards ctx asks the server to resolve the cross-file references that static name resolution left unresolved or ambiguous (see [Cross-file resolution](#cross-file-resolution)). ctx waits for the server to finish loading the workspace first (`ready_timeout_ms`, default 300 s), because servers such as rust-analyzer answer `null` until then:

```toml
[lsp.python]
command = "pyright-langserver"
args = ["--stdio"]
# backend = "hybrid" is the default; extensions default to ["py", "pyi"]
```

Without an `[lsp.*]` block, ctx starts no server and indexing works as described on this page.

## Rust

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `fn main()` | Top-level functions |
| Method | `fn new(&self)` | Functions in impl blocks |
| Struct | `struct User` | Struct definitions |
| Enum | `enum Status` | Enum definitions |
| Trait | `trait Display` | Trait definitions |
| Type alias | `type Result<T> = ...` | Type aliases |
| Const | `const MAX: i32` | Constants |
| Static | `static GLOBAL: &str` | Static variables |

### Visibility

- `pub` -> public
- `pub(crate)` -> crate
- `pub(super)` -> super
- No modifier -> private

### Example

```rust
/// A user in the system.
pub struct User {
    pub id: u64,
    pub name: String,
}

impl User {
    /// Create a new user.
    pub fn new(name: &str) -> Self {
        Self { 
            id: generate_id(),
            name: name.to_string() 
        }
    }
    
    /// Validate the user.
    fn validate(&self) -> bool {
        !self.name.is_empty()
    }
}

trait Authenticate {
    fn verify(&self) -> bool;
}

impl Authenticate for User {
    fn verify(&self) -> bool {
        self.validate()
    }
}
```

**Extracted symbols:**
- `User` (struct, public)
- `User::new` (method, public)
- `User::validate` (method, private)
- `Authenticate` (trait)
- `User::verify` (method, public via trait)

**Extracted edges:**
- `User::new` calls `generate_id`
- `User::new` calls `to_string`
- `User::verify` calls `validate`
- `User` implements `Authenticate`

Method-call syntax (`self.validate()`, `x.find(..)`) is never bound to a free function with the same name. A free function passed as a value (`spawn(worker::run_main)`, `.map(transform)`) becomes a `uses` edge when exactly one Rust function matches.

### Documentation

ctx extracts Rust doc comments (`///` and `//!`). The first sentence becomes `brief` and the full comment becomes `docstring`.

## TypeScript

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `function foo()` | Named functions |
| Function expression | `const foo = () => {}`, `var foo = function () {}` | Arrow functions and function expressions assigned with `const`, `let` or `var` |
| Property function | `exports.parse = () => ...`, `X.prototype.use = function () {}` | Functions assigned to a property; the symbol takes the property name |
| Class | `class User` | Class declarations |
| Method | `authenticate()` | Class methods |
| Interface | `interface IUser` | Interface declarations |
| Type Alias | `type UserId = string` | Type definitions |
| Enum | `enum Status` | Enum declarations |

### Visibility

- `export` -> public
- No `export` -> private
- `private` keyword in a class -> private
- `public` keyword in a class -> public
- `protected` keyword in a class -> protected

### Example

```typescript
/** User service for authentication. */
export interface UserService {
  authenticate(token: string): Promise<User>;
}

/** Default implementation. */
export class DefaultUserService implements UserService {
  constructor(private db: Database) {}
  
  async authenticate(token: string): Promise<User> {
    const decoded = decodeToken(token);
    return this.db.findUser(decoded.userId);
  }
  
  private validateToken(token: string): boolean {
    return token.length > 0;
  }
}

/** Decode a JWT token. */
export const decodeToken = (token: string): TokenPayload => {
  return jwt.decode(token);
};

type UserId = string;
```

**Extracted symbols:**
- `UserService` (interface, public)
- `DefaultUserService` (class, public)
- `DefaultUserService.authenticate` (method, public)
- `DefaultUserService.validateToken` (method, private)
- `decodeToken` (function, public)
- `UserId` (type, private)

**Extracted edges:**
- `DefaultUserService` implements `UserService`
- `authenticate` calls `decodeToken`
- `authenticate` calls `findUser`
- `decodeToken` calls `jwt.decode`

### Documentation

ctx extracts JSDoc comments (`/** */`). The first sentence becomes `brief` and the full comment becomes `docstring`.

## JavaScript and JSX

JavaScript gets the same extraction as TypeScript, without the type-only constructs (interfaces, type aliases, enums).

### JSX components

```jsx
// Function component
export function Button({ label, onClick }) {
  return <button onClick={onClick}>{label}</button>;
}

// Arrow function component
export const Card = ({ title, children }) => (
  <div className="card">
    <h2>{title}</h2>
    {children}
  </div>
);
```

**Extracted:**
- `Button` (function, public)
- `Card` (function, public)

## TSX

TSX combines TypeScript and JSX extraction:

```tsx
interface ButtonProps {
  label: string;
  onClick: () => void;
}

export const Button: React.FC<ButtonProps> = ({ label, onClick }) => {
  return <button onClick={onClick}>{label}</button>;
};
```

**Extracted:**
- `ButtonProps` (interface, private)
- `Button` (function, public)

## Python

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `def foo():` | Top-level functions |
| Async Function | `async def foo():` | Async functions |
| Class | `class User:` | Class definitions |
| Method | `def validate(self):` | Instance methods |
| Static Method | `@staticmethod def create():` | Static methods |
| Class Method | `@classmethod def from_dict(cls):` | Class methods |
| Constant | `MAX_RETRIES = 3` | UPPER_CASE names at module level |

### Visibility

- Names starting with `_` -> private
- Names starting with `__` (not `__dunder__`) -> private (name mangling)
- All other names -> public

### Example

```python
"""User module for authentication."""

MAX_RETRIES = 3

class User:
    """A user in the system."""
    
    def __init__(self, name: str):
        """Initialize the user."""
        self.name = name
    
    def validate(self) -> bool:
        """Validate the user."""
        return len(self.name) > 0
    
    @staticmethod
    def from_dict(data: dict) -> "User":
        """Create a user from a dictionary."""
        return User(data["name"])
    
    def _internal_check(self):
        """Private method."""
        pass

class Admin(User):
    """Admin user with elevated privileges."""
    
    def __init__(self, name: str, permissions: list):
        super().__init__(name)
        self.permissions = permissions

async def fetch_user(user_id: int) -> User:
    """Fetch a user from the database."""
    data = await db.get(user_id)
    return User.from_dict(data)
```

**Extracted symbols:**
- `MAX_RETRIES` (constant, public)
- `User` (class, public)
- `User.__init__` (method, private - starts with `_`)
- `User.validate` (method, public)
- `User.from_dict` (method, public)
- `User._internal_check` (method, private)
- `Admin` (class, public)
- `Admin.__init__` (method, private)
- `fetch_user` (function, public)

**Extracted edges:**
- `Admin` extends `User`
- `Admin.__init__` calls `super().__init__`
- `User.from_dict` calls `User`
- `fetch_user` calls `db.get`
- `fetch_user` calls `User.from_dict`

### Documentation

ctx extracts Python docstrings (triple-quoted strings). The first sentence becomes `brief` and the full docstring becomes `docstring`.

## Go

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `func Handle()` | Top-level functions |
| Method | `func (s *Server) Start()` | The receiver type is part of the ID and qualified name (`Server.Start`) |
| Struct | `type User struct {}` | Struct types |
| Interface | `type Reader interface {}` | Interface types |
| Type | `type ID string` | Other type definitions |
| Const | `const MaxRetries = 3` | Constants |
| Variable | `var defaultClient = ...` | Top-level variables |

Methods with the same name on different receivers are different symbols: `A.String` and `B.String` each have their own ID, callers and source.

### Visibility

- Exported identifiers (capitalized first letter) -> public
- Unexported identifiers (lowercase first letter) -> private

### Example

```go
package auth

// User represents an account in the system.
type User struct {
    ID   uint64
    Name string
}

// Authenticator verifies credentials.
type Authenticator interface {
    Verify(token string) (*User, error)
}

// NewUser creates a user with a generated ID.
func NewUser(name string) *User {
    return &User{ID: generateID(), Name: name}
}

func (u *User) validate() bool {
    return len(u.Name) > 0
}
```

**Extracted symbols:**
- `User` (struct, public)
- `Authenticator` (interface, public)
- `NewUser` (function, public)
- `User.validate` (method, private)

**Extracted edges:**
- `NewUser` calls `generateID`

Imports are recorded in module metadata.

### Documentation

ctx extracts Go doc comments (`//` lines directly above a declaration). The first sentence becomes `brief` and the full comment becomes `docstring`.

## C and C++

C and C++ files are fully indexed. Ambiguous `.h` files are parsed with both
grammars; the tree with fewer error and missing nodes is selected, with C
winning ties. The selected `c` or `cpp` language is stored in the index and is
also used for structural fingerprints.

Definitions and prototypes, methods, constructors, destructors, operators,
structs, classes, unions, enums, namespaces, typedefs, aliases, top-level
variables/constants, and object- and function-like macros are searchable.
C++ symbols receive `::`-qualified names and syntactic namespace/class parent
links. C `static` declarations and declarations in anonymous C++ namespaces
are private. C++ class/struct access defaults and access specifiers are
honored; `protected` maps to private because ctx has no protected visibility.
Contiguous `///`/`//!` comments and Doxygen blocks provide documentation.

Direct, member, qualified, and constructor syntax produces call edges.
Quoted includes retain the unquoted path and resolve from the importing
directory, repository root, or a unique indexed suffix. Angle includes retain
`<...>` and resolve only when the enclosed path exactly matches an indexed
repository path. System, missing, ambiguous, computed, root-escaping, and
build-path-dependent includes remain unresolved; `compile_commands.json`,
type inference, overload resolution, template instantiation, and preprocessor
expansion are outside static analysis.

During indexing, a call to a function that is declared in a header and defined
in exactly one `.c`/`.cc` file is bound to the definition. `ctx judge edges`
does not offer a header prototype as a candidate next to its own definition.

C/C++ functions participate in `ctx duplicates`; identifiers and literals are
normalized and comments are removed.

## Zig

Zig `.zig` files are fully indexed. `.zon` files stay ordinary context
content and are not parsed as Zig source.

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `fn run() void` | Named top-level functions |
| Method | `fn start(self: *Server) void` | Functions in named containers |
| Struct | `const Server = struct {}` | Named struct initializers |
| Enum | `const Mode = enum { fast }` | Named enum initializers |
| Type | `const Value = union(enum) { ... }` | Union, opaque, and error-set initializers |
| Const / Variable | `const limit = 10`, `var count = 0` | Container-level declarations |
| Named test | `test "parses input" {}` | Anonymous test blocks are skipped |

`pub` declarations are public; other declarations are private. Contiguous
`///` comments become the symbol's `docstring`, with the first line used as
its `brief`. Container fields are not indexed as symbols.

Identifier calls, field calls such as `server.start()`, and builtin calls such
as `@panic()` produce call edges. Literal imports record their alias and
specifier: `const util = @import("../util.zig")`. Imports ending in `.zig` are
resolved relative to the importing file for architecture checks. Package
imports such as `@import("std")`, missing files, dynamic imports, and paths
escaping the index root remain unresolved.

Zig functions participate in `ctx duplicates`; identifiers and literals are
normalized and comments are removed, as for the other tree-sitter languages.
Static analysis does not perform type inference, computed-call resolution,
comptime expansion, or Zig package-manager dependency resolution.

## Solidity

### Extracted symbols

| Kind | Example | Notes |
|------|---------|-------|
| Contract | `contract Token` | Contract definitions |
| Function | `function transfer()` | Contract functions |
| Modifier | `modifier onlyOwner` | Function modifiers |
| Event | `event Transfer` | Event definitions |
| Struct | `struct Proposal` | Struct definitions |
| Enum | `enum Status` | Enum definitions |
| Error | `error Unauthorized` | Custom errors |

### Visibility

- `public` -> public
- `external` -> public
- `internal` -> crate (treated as internal)
- `private` -> private

### Example

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title Token contract
/// @notice Implements ERC20-like functionality
contract Token {
    mapping(address => uint256) public balances;
    
    event Transfer(address indexed from, address indexed to, uint256 amount);
    
    error InsufficientBalance(uint256 available, uint256 required);
    
    modifier onlyPositive(uint256 amount) {
        require(amount > 0, "Amount must be positive");
        _;
    }
    
    /// @notice Transfer tokens to another address
    /// @param to Recipient address
    /// @param amount Amount to transfer
    function transfer(address to, uint256 amount) 
        external 
        onlyPositive(amount) 
    {
        if (balances[msg.sender] < amount) {
            revert InsufficientBalance(balances[msg.sender], amount);
        }
        balances[msg.sender] -= amount;
        balances[to] += amount;
        emit Transfer(msg.sender, to, amount);
    }
}
```

**Extracted symbols:**
- `Token` (contract, public)
- `Token.transfer` (function, external)
- `Token.onlyPositive` (modifier)
- `Transfer` (event)
- `InsufficientBalance` (error)

**Extracted edges:**
- `Token.transfer` calls `Token.onlyPositive`. Modifier applications produce `calls` edges, so `ctx query callers`, `ctx query impact` and `v1.edges` can answer access-control questions. Constructor base-contract invocations produce `calls` edges too.
- Qualified library calls (`Lib.fn()`) resolve to the library function.
- A `using Lib for Type` directive produces a `uses` edge from the contract.

### Documentation

ctx extracts NatSpec comments (`///` and `/** */`). The first sentence becomes `brief` and the full comment becomes `docstring`.

## YAML

ctx tracks YAML files but does not parse them for symbols. The index stores the file path, content hash and language, so YAML files appear in `ctx query files` and in context generation.

## Edge types

| Edge type | Languages | Description |
|-----------|-----------|-------------|
| `calls` | All parsed languages | Function and method calls |
| `extends` | TypeScript, JavaScript, Python | Class inheritance |
| `implements` | TypeScript, Rust | Interface or trait implementation |
| `uses` | Rust, Solidity | Rust function values passed as arguments; Solidity `using ... for` |

Imports and includes are module metadata, not edges (see [Supported languages](#supported-languages)).

## Limitations

### Cross-file resolution

ctx resolves call targets by name within the indexed codebase. Calls into libraries stay unresolved:

```
ctx query deps myFunction
Dependencies of 'myFunction':
------------------------------------------------------------
  calls internalHelper (line 12)      # Resolved
  calls externalLib (line 15)         # Unresolved (external)
```

Two opt-in steps resolve more edges:

- A language server with `backend = "hybrid"` resolves references that stayed unresolved after tree-sitter extraction, using `textDocument/definition` (see [Hybrid mode for built-in languages](#hybrid-mode-for-built-in-languages)).
- `ctx judge edges` asks a decision model to pick the real callee among same-named candidates, or `external`. See [ctx judge](commands/judge.md).

### Dynamic calls

ctx can't track dynamic or computed calls:

```typescript
// Static call - tracked
processData(input);

// Dynamic call - not tracked
const fn = getHandler(type);
fn(input);

// Not tracked
obj[methodName]();
```

### Macros (Rust)

ctx does not analyze macro-generated code:

```rust
// The generated impl is not indexed
#[derive(Debug, Clone)]
struct MyStruct { ... }

// Macro invocations are not followed
println!("Hello");  // Not tracked as a call
```

### Decorators (Python)

ctx tracks decorated functions but not the decorator calls:

```python
@cache  # Not tracked as a call
def expensive_operation():
    pass
```

### Type inference

ctx does not infer types, so method calls on values with inferred types may not resolve:

```typescript
// Resolved (explicit type)
const user: User = getUser();
user.validate();  // Knows validate is on User

// May not resolve (inferred type)
const user = getUser();
user.validate();  // ctx doesn't know user is a User
```

### Indirect calls

ctx does not record calls through variables, closures or dynamic dispatch.
A Rust free function passed directly as a callback value is recorded as a
`uses` edge when exactly one Rust function matches:

```rust
// Dynamic invocation through a variable is not tracked
let callback = process_data;
callback(input);

// A direct Rust function item is a `uses` edge, not a call
[1, 2, 3].into_iter().map(transform);
```

## Adding language support

The fastest way to add a language is to register its language server in `.ctx/config.toml`, with no code changes. See [Add a language via LSP](lsp-languages.md).

To add built-in tree-sitter support for a new language:

1. Add the tree-sitter crate to `Cargo.toml`:
   ```toml
   tree-sitter-newlang = "0.x"
   ```

2. Create a parser module in `src/parser/`:
   ```rust
   // src/parser/newlang.rs
   pub struct NewLangParser { ... }
   impl NewLangParser {
       pub fn parse(&mut self, file_path: &str, source: &str) -> Option<ParseResult>;
   }
   ```

3. Define tree-sitter queries for symbol extraction

4. Add the language to the `Language` enum in `src/parser/mod.rs`

5. Update `is_supported()` in `CodeParser`

See `src/parser/rust.rs` or `src/parser/typescript.rs` for examples.

## Language detection

ctx detects the language from the file extension:

```rust
match extension {
    "rs" => Rust,
    "ts" => TypeScript,
    "tsx" => Tsx,
    "js" | "mjs" | "cjs" => JavaScript,
    "jsx" => Jsx,
    "py" | "pyi" => Python,
    "sol" => Solidity,
    "yaml" | "yml" => Yaml,
    "go" => Go,
    "c" | "h" => C, // `.h` is refined by parsing with C and C++
    "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "ipp" | "tpp" => Cpp,
    "zig" => Zig,
    _ => Unknown,
}
```

Indexing skips unknown extensions, but context generation includes them. If an `[lsp.<language>]` block in `.ctx/config.toml` claims an extension, ctx indexes those files through the registered language server.
