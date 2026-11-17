# Editor support

`neuron lsp` is a language server (Language Server Protocol 3.17) that any editor can
start. It gives:

| Feature | What you see |
|---|---|
| Diagnostics | the errors and warnings of `neuron check`, while you type, for every file of the package |
| Hover | the declaration and documentation of a name, and the type (with its unit) of an expression |
| Go to definition | the declaration of a function, signal, task, I/O binding or type |
| Formatting | the canonical layout of `neuron fmt` |

The server analyzes every `.neuron` file under the workspace folder (with the text of the
open files), and reads `neuron.toml` from the folder when there is one.

## Visual Studio Code and Kiro

Kiro is built on Code OSS, so the same extension works in both. The extension is in
`editors/vscode/`: a TextMate grammar (colors), the language configuration (comments,
brackets, indentation) and a client that starts `neuron lsp`.

1. Build the tool and put it on the path: `cargo install --path crates/neuron-cli`
   (or set the setting `neuron.path` to `target/release/neuron`).
2. In `editors/vscode/` run `npm install`, then `npx @vscode/vsce package`.
3. Install the generated `.vsix`: *Extensions → … → Install from VSIX*.
4. Open a folder with `.neuron` files.

The colors work without the tool; diagnostics, hover and formatting need `neuron` on the
path.

## Other editors

Any editor with LSP support only needs the command `neuron lsp` on standard input and
output for files of type `neuron` (extension `.neuron`).

## Command line

```text
neuron fmt [--check] [--stdin] <file|dir>...   canonical layout
neuron explain NE0201                          long explanation of a diagnostic
neuron lsp                                     language server
```

`neuron fmt --check` is what CI runs on `examples/`.
