"use strict";

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

/** @type {LanguageClient | undefined} */
let client;

function activate(context) {
  const command = vscode.workspace.getConfiguration("neuron").get("path", "neuron");
  const serverOptions = {
    command,
    args: ["lsp"],
    transport: TransportKind.stdio,
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "neuron" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/{*.neuron,neuron.toml}"),
    },
  };
  client = new LanguageClient("neuron", "Neuron Language Server", serverOptions, clientOptions);
  context.subscriptions.push({ dispose: () => deactivate() });
  client.start();
}

function deactivate() {
  if (!client) {
    return undefined;
  }
  const stopping = client.stop();
  client = undefined;
  return stopping;
}

module.exports = { activate, deactivate };
