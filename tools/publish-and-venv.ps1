# Commits and pushes current work to develop, then rebuilds the Python venv.
# Logs: out/git-publish.log and out/venv.log.
$root = Split-Path -Parent $PSScriptRoot
powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'tools/git-publish.ps1') -Branch develop -Message "feat(lang): lexer, AST and parser for Neuron with property tests; language spec v0.1 and EBNF grammar"
powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'tools/rebuild-venv.ps1')
