# Printing Press

A standalone Rust compiler for the [Inklang](https://github.com/inklang/inklang) scripting language. Compiles `.ink` source files to JSON bytecode for execution by the Inklang VM.

## Usage

```bash
# Compile a script
printing_press compile script.ink -o script.json

# Compile with debug output (pretty JSON)
printing_press compile script.ink -o script.json --debug

# Show version
printing_press --version

# Show help
printing_press --help
```

## Installation

```bash
cargo install --path .
```

## Architecture

```
Source (.ink)
    │
    ▼
Lexer (tokenize) → Token stream
    │
    ▼
Parser (Pratt) → AST
    │
    ▼
ConstantFolder → Optimized AST
    │
    ▼
AstLowerer → IR instructions
    │
    ▼
SSA Round-trip (optimize)
    │
    ▼
LivenessAnalyzer → Live ranges
    │
    ▼
RegisterAllocator → Physical register map
    │
    ▼
SpillInserter → Insert SPILL/UNSPILL
    │
    ▼
IrCompiler → Bytecode Chunk
    │
    ▼
Serialize → JSON (.ink.json)
```

## Output Format

The compiler outputs JSON matching the `SerialScript` schema. See [serialization spec](docs/serialization.md) for details.

## Status

**In development.** The compiler pipeline works for many inputs but has known issues with certain SSA patterns. 187 unit tests pass.

## License

MIT
