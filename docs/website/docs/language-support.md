---
id: language-support
title: Language Support
sidebar_position: 6
---

# Language Support

ctx uses tree-sitter for parsing, providing accurate symbol extraction across multiple programming languages. This page details what's extracted from each language and how relationships are tracked.

## Supported Languages Overview

| Language | Extensions | Symbol Extraction | Edge Tracking | Status |
|----------|-----------|-------------------|---------------|--------|
| Rust | `.rs` | Full | Calls, Implements, Imports | Full |
| TypeScript | `.ts` | Full | Calls, Extends, Implements, Imports | Full |
| TSX | `.tsx` | Full | Calls, Extends, Implements, Imports | Full |
| JavaScript | `.js`, `.mjs`, `.cjs` | Full | Calls, Extends, Imports | Full |
| JSX | `.jsx` | Full | Calls, Extends, Imports | Full |
| Python | `.py`, `.pyi` | Full | Calls, Extends, Imports | Full |
| Go | `.go` | Full | Calls, Imports | Full |
| Solidity | `.sol` | Full | Calls | Full |
| YAML | `.yaml`, `.yml` | File tracking only | N/A | Partial |

## Rust

### Extracted Symbols

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

### Visibility Detection

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

### Documentation Extraction

Rust doc comments (`///` and `//!`) are extracted:
- First line becomes the `brief` field
- Full content becomes `docstring`

## TypeScript

### Extracted Symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `function foo()` | Named functions |
| Arrow Function | `const foo = () => {}` | With const declaration |
| Class | `class User` | Class declarations |
| Method | `authenticate()` | Class methods |
| Interface | `interface IUser` | Interface declarations |
| Type Alias | `type UserId = string` | Type definitions |
| Enum | `enum Status` | Enum declarations |

### Visibility Detection

- `export` -> public
- No `export` -> private
- `private` keyword in class -> private
- `public` keyword in class -> public
- `protected` keyword in class -> protected

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

### JSDoc Extraction

JSDoc comments (`/** */`) are extracted:
- `@description` or first line -> `brief`
- Full content -> `docstring`

## JavaScript / JSX

Same as TypeScript, minus type-specific constructs (interfaces, type aliases, enums).

### Additional JSX Support

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

Combines TypeScript and JSX support:

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

### Extracted Symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `def foo():` | Top-level functions |
| Async Function | `async def foo():` | Async functions |
| Class | `class User:` | Class definitions |
| Method | `def validate(self):` | Instance methods |
| Static Method | `@staticmethod def create():` | Static methods |
| Class Method | `@classmethod def from_dict(cls):` | Class methods |
| Constant | `MAX_RETRIES = 3` | UPPER_CASE names at module level |

### Visibility Detection

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

### Docstring Extraction

Python docstrings (triple-quoted strings) are extracted:
- First line -> `brief`
- Full content -> `docstring`

## Go

### Extracted Symbols

| Kind | Example | Notes |
|------|---------|-------|
| Function | `func Handle()` | Top-level functions |
| Method | `func (s *Server) Start()` | Methods with receivers |
| Struct | `type User struct {}` | Struct type definitions |
| Interface | `type Reader interface {}` | Interface type definitions |
| Const | `const MaxRetries = 3` | Constants |

### Visibility Detection

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

### Documentation Extraction

Go doc comments (`//` immediately preceding a declaration) are extracted:
- First line becomes the `brief` field
- Full comment becomes `docstring`

## Solidity

### Extracted Symbols

| Kind | Example | Notes |
|------|---------|-------|
| Contract | `contract Token` | Contract definitions |
| Function | `function transfer()` | Contract functions |
| Modifier | `modifier onlyOwner` | Function modifiers |
| Event | `event Transfer` | Event definitions |
| Struct | `struct Proposal` | Struct definitions |
| Enum | `enum Status` | Enum definitions |
| Error | `error Unauthorized` | Custom errors |

### Visibility Detection

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
- `Token.transfer` -> `Token.onlyPositive` (calls) — **modifier applications emit `calls` edges**, so `ctx query callers`/`impact` and `v1.edges` answer access-control questions; constructor base-contract invocations are covered too.
- Qualified library calls (`Lib.fn()`) resolve to the library function instead of remaining unresolved in the call graph.

### NatSpec Extraction

NatSpec comments (`///` and `/** */`) are extracted:
- `@title` or `@notice` -> `brief`
- Full content -> `docstring`

## YAML

YAML files are tracked but not parsed for symbols (YAML doesn't have functions/classes).

**What's indexed:**
- File path
- File hash (for change detection)
- Language type

**Use cases:**
- Include config files in `ctx query files`
- Track changes to CI/CD configs
- Include in context generation for reference

## Edge Types Summary

| Edge Type | Languages | Description |
|-----------|-----------|-------------|
| `calls` | All | Function/method calls |
| `extends` | TS, JS, Python | Class inheritance |
| `implements` | TS, Rust | Interface/trait implementation |
| `imports` | All (except Solidity) | Module imports |

## Limitations

### Cross-File Resolution

Call targets are resolved by name within the same codebase. External calls (to libraries) show as unresolved:

```
ctx query deps myFunction
Dependencies of 'myFunction':
------------------------------------------------------------
  calls internalHelper (line 12)      # Resolved
  calls externalLib (line 15)         # Unresolved (external)
```

### Dynamic Calls

Dynamic or computed calls cannot be tracked:

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

Macro-generated code is not analyzed:

```rust
// The generated impl is not indexed
#[derive(Debug, Clone)]
struct MyStruct { ... }

// Macro invocations are not followed
println!("Hello");  // Not tracked as a call
```

### Decorators (Python)

Decorated functions are tracked, but decorator calls are not:

```python
@cache  # Not tracked as a call
def expensive_operation():
    pass
```

### Type Inference

We don't perform type inference, so method calls on inferred types may not resolve:

```typescript
// Resolved (explicit type)
const user: User = getUser();
user.validate();  // Knows validate is on User

// May not resolve (inferred type)
const user = getUser();
user.validate();  // We don't know user is User
```

### Indirect Calls

Calls through variables, callbacks, or higher-order functions are not tracked:

```javascript
// Not tracked
const callback = processData;
callback(input);

// Not tracked
[1, 2, 3].map(transform);
```

## Adding Language Support

To add support for a new language:

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

## Language Detection

ctx detects language by file extension:

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
    _ => Unknown,
}
```

Unknown extensions are skipped during indexing but included in context generation.
