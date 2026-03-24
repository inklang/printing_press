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

## CLI Reference

### Single-File Compilation

```bash
printing_press compile <input.ink> -o <output.json> [--debug]
```

### Batch Compilation

```bash
printing_press compile --sources <dir> --out <dir> [--debug]
```

Scan a directory of `.ink` files and compile each to `.inkc`. Grammars are auto-discovered from `dist/grammar.ir.json` and `packages/*/dist/grammar.ir.json`.

**Arguments:**
- `--sources <dir>` — Directory containing `.ink` source files
- `--out <dir>` — Output directory for compiled `.inkc` files
- `--debug` — Pretty-print JSON output
- `-o <file>` — Output file (single-file mode only)

**Grammar Auto-Discovery:**
The compiler scans for grammars in:
1. `dist/grammar.ir.json` (project's own grammar)
2. `packages/*/dist/grammar.ir.json` (installed packages)

No `--grammar` flags needed — grammars are discovered automatically when the compiler runs from the project root directory.

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
