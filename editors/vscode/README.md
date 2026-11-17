# Neuron for VS Code and Kiro

Syntax highlighting plus a language client for `neuron lsp` (diagnostics, hover,
go to definition, formatting).

## Install (from source)

```powershell
cargo install --path crates/neuron-cli      # puts `neuron` on PATH
cd editors/vscode
npm install
npx @vscode/vsce package                    # builds neuron-language-0.1.0.vsix
code --install-extension neuron-language-0.1.0.vsix   # or: kiro --install-extension ...
```

If `neuron` is not on `PATH`, set the `neuron.path` setting to the executable.

See `docs/editor-support.md` for the supported LSP features and the limits of the
formatter.
