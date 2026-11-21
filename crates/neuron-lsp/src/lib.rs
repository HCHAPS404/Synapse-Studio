//! A small language server for Neuron (Language Server Protocol 3.17, JSON-RPC over
//! standard input and output).
//!
//! It offers diagnostics (from the full analysis of the package), hover (the declaration
//! of a name and the type of an expression), go to definition and document formatting.
//! It analyzes the whole workspace on every change: every `.neuron` file under the root,
//! with the text of the open documents in place of the files on disk.
//!
//! ```
//! use neuron_lsp::Server;
//! use serde_json::json;
//!
//! let mut s = Server::new();
//! let out = s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
//! assert_eq!(out[0]["result"]["capabilities"]["hoverProvider"], json!(true));
//! ```

mod transport;

pub use transport::serve;

use neuron_nir::Nir;
use neuron_sema::{analyze, Analysis, SemaOptions, SourceFile};
use neuron_syntax::format::format_source;
use neuron_syntax::lexer::lex;
use neuron_syntax::outline::outline;
use neuron_syntax::{parse, TokenKind};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The state of one language server session.
#[derive(Default)]
pub struct Server {
    root: Option<String>,
    /// Open documents by normalized path: client URI and text.
    docs: BTreeMap<String, (String, String)>,
    shutdown: bool,
    exit: bool,
    published: BTreeSet<String>,
    last: Option<Analysis>,
    uris: Vec<String>,
}

/// Path with `/` separators and a lower-case drive letter, so that paths compare equal.
fn norm_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        let first = s[..1].to_ascii_lowercase();
        s.replace_range(..1, &first);
    }
    s
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// File path of a `file://` URI.
fn uri_to_path(uri: &str) -> String {
    let rest = uri.strip_prefix("file://").unwrap_or(uri);
    let p = percent_decode(rest);
    let p = if p.len() > 2 && p.as_bytes()[0] == b'/' && p.as_bytes()[2] == b':' {
        p[1..].to_string()
    } else {
        p
    };
    norm_path(&p)
}

/// `file://` URI of a path, written the way editors write it.
fn path_to_uri(path: &str) -> String {
    let p = norm_path(path);
    let enc: String = p
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            ':' => "%3A".to_string(),
            c => c.to_string(),
        })
        .collect();
    if enc.starts_with('/') {
        format!("file://{enc}")
    } else {
        format!("file:///{enc}")
    }
}

fn offset_of(text: &str, line: usize, ch: usize) -> usize {
    let mut start = 0;
    for (i, l) in text.split('\n').enumerate() {
        if i == line {
            let off: usize = l.chars().take(ch).map(char::len_utf8).sum();
            return start + off.min(l.len());
        }
        start += l.len() + 1;
    }
    text.len()
}

fn position_of(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count());
    (line, col)
}

fn range(text: &str, start: usize, end: usize) -> Value {
    let (sl, sc) = position_of(text, start);
    let (el, ec) = position_of(text, end);
    json!({"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}})
}

/// Text of a resolved type, for hover.
fn type_text(ty: &Value) -> Option<String> {
    let t = ty.get("t")?.as_str()?;
    Some(match t {
        "unknown" => return None,
        "num" => {
            let kind = ty.get("kind").and_then(Value::as_str).unwrap_or("number");
            match ty.pointer("/unit/text").and_then(Value::as_str) {
                Some(u) => format!("{kind} {u}"),
                None => kind.to_string(),
            }
        }
        "adt" | "interface" | "opaque" => ty.get("name").and_then(Value::as_str)?.to_string(),
        "io" => format!(
            "io {}",
            ty.get("kind").and_then(Value::as_str).unwrap_or("")
        ),
        other => other.to_string(),
    })
}

/// The smallest expression of the NIR that contains `offset`, as `(span, type text)`.
fn smallest_expr(v: &Value, offset: usize, best: &mut Option<(usize, String)>) {
    match v {
        Value::Object(m) => {
            if let (Some(span), Some(ty)) = (m.get("span").and_then(Value::as_array), m.get("ty")) {
                let s = span.first().and_then(Value::as_u64).unwrap_or(0) as usize;
                let e = span.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                let len = e.saturating_sub(s);
                if s <= offset && offset < e.max(s + 1) {
                    if let Some(t) = type_text(ty) {
                        if best.as_ref().is_none_or(|(l, _)| len <= *l) {
                            *best = Some((len, t));
                        }
                    }
                }
            }
            for x in m.values() {
                smallest_expr(x, offset, best);
            }
        }
        Value::Array(a) => {
            for x in a {
                smallest_expr(x, offset, best);
            }
        }
        _ => {}
    }
}

impl Server {
    /// A session that has not been initialized.
    pub fn new() -> Server {
        Server::default()
    }

    /// `true` after the `exit` notification.
    pub fn exit_requested(&self) -> bool {
        self.exit
    }

    /// `true` when `shutdown` was requested before `exit`.
    pub fn was_shut_down(&self) -> bool {
        self.shutdown
    }

    /// Handles one message from the client and returns the messages to send back.
    pub fn handle(&mut self, msg: &Value) -> Vec<Value> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let reply = |result: Value| -> Vec<Value> {
            match &id {
                Some(i) => vec![json!({"jsonrpc": "2.0", "id": i, "result": result})],
                None => Vec::new(),
            }
        };
        match method {
            "initialize" => {
                self.root = params
                    .get("rootUri")
                    .and_then(Value::as_str)
                    .map(uri_to_path)
                    .or_else(|| {
                        params
                            .get("rootPath")
                            .and_then(Value::as_str)
                            .map(norm_path)
                    });
                reply(json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "documentFormattingProvider": true
                    },
                    "serverInfo": {"name": "neuron-lsp", "version": env!("CARGO_PKG_VERSION")}
                }))
            }
            "initialized" => self.refresh(),
            "shutdown" => {
                self.shutdown = true;
                reply(Value::Null)
            }
            "exit" => {
                self.exit = true;
                Vec::new()
            }
            "textDocument/didOpen" => {
                let d = &params["textDocument"];
                if let (Some(uri), Some(text)) = (d["uri"].as_str(), d["text"].as_str()) {
                    self.docs
                        .insert(uri_to_path(uri), (uri.to_string(), text.to_string()));
                }
                self.refresh()
            }
            "textDocument/didChange" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let text = params["contentChanges"]
                    .as_array()
                    .and_then(|c| c.last())
                    .and_then(|c| c["text"].as_str());
                if let Some(text) = text {
                    self.docs
                        .insert(uri_to_path(uri), (uri.to_string(), text.to_string()));
                }
                self.refresh()
            }
            "textDocument/didSave" => self.refresh(),
            "textDocument/didClose" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                self.docs.remove(&uri_to_path(uri));
                self.refresh()
            }
            "textDocument/hover" => reply(self.hover(&params).unwrap_or(Value::Null)),
            "textDocument/definition" => reply(self.definition(&params).unwrap_or(Value::Null)),
            "textDocument/formatting" => reply(self.formatting(&params)),
            _ if id.is_some() => vec![json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method not found: {method}")}
            })],
            _ => Vec::new(),
        }
    }

    // ---- analysis ---------------------------------------------------------------

    fn disk_files(&self) -> BTreeMap<String, (String, String)> {
        let mut out = BTreeMap::new();
        if let Some(root) = &self.root {
            let mut stack = vec![Path::new(root).to_path_buf()];
            while let Some(dir) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for e in rd.flatten() {
                    let p = e.path();
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if p.is_dir() {
                        if !matches!(name.as_str(), "target" | ".git" | "node_modules" | "out") {
                            stack.push(p);
                        }
                    } else if p.extension().is_some_and(|x| x == "neuron") {
                        if let Ok(text) = std::fs::read_to_string(&p) {
                            let path = norm_path(&p.to_string_lossy());
                            out.insert(path.clone(), (path_to_uri(&path), text));
                        }
                    }
                }
            }
        }
        out
    }

    /// Analyzes the workspace and returns the `publishDiagnostics` notifications.
    fn refresh(&mut self) -> Vec<Value> {
        let mut files = self.disk_files();
        for (p, v) in &self.docs {
            files.insert(p.clone(), v.clone());
        }
        let manifest = self
            .root
            .as_ref()
            .and_then(|r| std::fs::read_to_string(Path::new(r).join("neuron.toml")).ok());
        let sources: Vec<SourceFile> = files
            .iter()
            .map(|(p, (_, t))| SourceFile {
                path: p.clone(),
                text: t.clone(),
            })
            .collect();
        self.uris = files.values().map(|(u, _)| u.clone()).collect();
        let a = analyze(sources, manifest.as_deref(), &SemaOptions::default());
        let mut by_uri: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for fd in &a.diagnostics {
            let text = &a.texts[fd.file];
            let d = &fd.diag;
            let mut message = d.message.clone();
            if let Some(h) = &d.help {
                message.push_str("\nhelp: ");
                message.push_str(h);
            }
            by_uri
                .entry(self.uris[fd.file].clone())
                .or_default()
                .push(json!({
                    "range": range(text, d.span.start, d.span.end.max(d.span.start + 1)),
                    "severity": if d.is_error() { 1 } else { 2 },
                    "code": d.code,
                    "source": "neuron",
                    "message": message
                }));
        }
        let mut out = Vec::new();
        let stale: Vec<String> = self
            .published
            .iter()
            .filter(|u| !by_uri.contains_key(*u))
            .cloned()
            .collect();
        for uri in stale {
            out.push(publish(&uri, Vec::new()));
            self.published.remove(&uri);
        }
        for (uri, diags) in by_uri {
            self.published.insert(uri.clone());
            out.push(publish(&uri, diags));
        }
        for (uri, _) in self.docs.values() {
            if !self.published.contains(uri) {
                out.push(publish(uri, Vec::new()));
            }
        }
        self.last = Some(a);
        out
    }

    fn file_index(&self, uri: &str) -> Option<usize> {
        let a = self.last.as_ref()?;
        let p = uri_to_path(uri);
        a.paths.iter().position(|x| *x == p)
    }

    fn name_at(text: &str, offset: usize) -> Option<(String, usize, usize)> {
        name_at(text, offset)
    }

    /// Declaration of `name` in any file: `(file index, line, header text, doc comment)`.
    fn declaration(&self, name: &str) -> Option<(usize, usize, String, String)> {
        find_declaration(&self.last.as_ref()?.texts, name)
    }

    fn doc_text(&self, uri: &str) -> Option<(usize, String)> {
        let i = self.file_index(uri)?;
        Some((i, self.last.as_ref()?.texts[i].clone()))
    }

    fn hover(&self, params: &Value) -> Option<Value> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let (fi, text) = self.doc_text(uri)?;
        let line = params["position"]["line"].as_u64()? as usize;
        let ch = params["position"]["character"].as_u64()? as usize;
        let off = offset_of(&text, line, ch);
        let (name, s, e) = Self::name_at(&text, off)?;
        let mut parts: Vec<String> = Vec::new();
        if let Some((_, _, header, doc)) = self.declaration(&name) {
            parts.push(format!("```neuron\n{header}\n```"));
            if !doc.is_empty() {
                parts.push(doc);
            }
        }
        if let Some(nir) = self.last.as_ref().and_then(|a| a.nir.as_ref()) {
            if let Some(ty) = Self::type_at(nir, fi, off) {
                parts.push(format!("type: `{ty}`"));
            }
        }
        if parts.is_empty() {
            return None;
        }
        Some(json!({
            "contents": {"kind": "markdown", "value": parts.join("\n\n")},
            "range": range(&text, s, e)
        }))
    }

    fn type_at(nir: &Nir, module: usize, off: usize) -> Option<String> {
        type_at(nir, module, off)
    }

    fn definition(&self, params: &Value) -> Option<Value> {
        let uri = params["textDocument"]["uri"].as_str()?;
        let (_, text) = self.doc_text(uri)?;
        let line = params["position"]["line"].as_u64()? as usize;
        let ch = params["position"]["character"].as_u64()? as usize;
        let off = offset_of(&text, line, ch);
        let (name, _, _) = Self::name_at(&text, off)?;
        let (fi, dline, header, _) = self.declaration(&name)?;
        let target = self.uris.get(fi)?.clone();
        Some(json!({
            "uri": target,
            "range": {
                "start": {"line": dline, "character": 0},
                "end": {"line": dline, "character": header.chars().count()}
            }
        }))
    }

    fn formatting(&self, params: &Value) -> Value {
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return json!([]);
        };
        let Some((_, text)) = self.docs.get(&uri_to_path(uri)).cloned() else {
            return json!([]);
        };
        match format_source(&text) {
            Ok(f) if f.text != text => {
                let end = position_of(&text, text.len());
                json!([{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": end.0, "character": end.1}},
                    "newText": f.text
                }])
            }
            _ => json!([]),
        }
    }
}

/// The identifier or keyword token that contains `offset`: `(text, start, end)` in bytes.
///
/// Transport-free helper shared with the WASM and Python bindings (`neuron-wasm`).
pub fn name_at(text: &str, offset: usize) -> Option<(String, usize, usize)> {
    lex(text)
        .tokens
        .iter()
        .find(|t| {
            matches!(t.kind, TokenKind::Ident | TokenKind::Keyword(_))
                && t.span.start <= offset
                && offset <= t.span.end
        })
        .map(|t| (t.text(text).to_string(), t.span.start, t.span.end))
}

/// Declaration of `name` in any of `texts`: `(file index, 0-based line, header text, doc comment)`.
pub fn find_declaration(texts: &[String], name: &str) -> Option<(usize, usize, String, String)> {
    for (i, text) in texts.iter().enumerate() {
        let p = parse(text);
        let lines: Vec<&str> = text.split('\n').collect();
        for l in outline(&p.file, text).lines().skip(1) {
            let mut words = l.split_whitespace();
            let (Some(num), Some(_kind)) = (words.next(), words.next()) else {
                continue;
            };
            let mut next = words.next();
            if next == Some("pub") {
                next = words.next();
            }
            let decl: String = next
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if decl != name {
                continue;
            }
            let mut line = num.parse::<usize>().ok()?.saturating_sub(1);
            // An item may start at its documentation or its attributes.
            while lines.get(line).is_some_and(|l| {
                l.trim_start().starts_with("##") || l.trim_start().starts_with('@')
            }) {
                line += 1;
            }
            let header = lines.get(line).copied().unwrap_or("").trim().to_string();
            let mut doc = Vec::new();
            let mut k = line;
            while k > 0 && lines[k - 1].trim_start().starts_with("##") {
                k -= 1;
                doc.push(
                    lines[k]
                        .trim_start()
                        .trim_start_matches('#')
                        .trim()
                        .to_string(),
                );
            }
            doc.reverse();
            return Some((i, line, header, doc.join("\n")));
        }
    }
    None
}

/// Type text of the smallest typed expression of module `module` that contains `off`.
pub fn type_at(nir: &Nir, module: usize, off: usize) -> Option<String> {
    let m = nir.modules.get(module)?;
    let v = serde_json::to_value(m).ok()?;
    let mut best = None;
    smallest_expr(&v, off, &mut best);
    best.map(|(_, t)| t)
}

fn publish(uri: &str, diagnostics: Vec<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {"uri": uri, "diagnostics": diagnostics}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(s: &mut Server, uri: &str, text: &str) -> Vec<Value> {
        s.handle(&json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {"textDocument": {"uri": uri, "languageId": "neuron", "version": 1, "text": text}}
        }))
    }

    const URI: &str = "file:///proj/a.neuron";

    #[test]
    fn uris_and_paths_round_trip() {
        assert_eq!(
            uri_to_path("file:///c%3A/Users/a%20b/x.neuron"),
            "c:/Users/a b/x.neuron"
        );
        assert_eq!(uri_to_path("file:///home/u/x.neuron"), "/home/u/x.neuron");
        assert_eq!(
            path_to_uri("C:\\Users\\a b\\x.neuron"),
            "file:///c%3A/Users/a%20b/x.neuron"
        );
    }

    #[test]
    fn errors_are_published_as_diagnostics() {
        let mut s = Server::new();
        let out = open(&mut s, URI, "module t\nfn f() -> u8:\n  return nope\n");
        let d = &out[0]["params"]["diagnostics"][0];
        assert_eq!(d["code"], "NE0101");
        assert_eq!(d["severity"], 1);
        assert_eq!(d["range"]["start"]["line"], 2);
        // Fixing the file clears them.
        let out = s.handle(&json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {"textDocument": {"uri": URI}, "contentChanges": [{"text": "module t\nfn f() -> u8:\n  return 1\n"}]}
        }));
        assert!(out.iter().any(|m| m["params"]["uri"] == URI
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(Vec::is_empty)));
    }

    #[test]
    fn hover_and_definition_find_the_declaration() {
        let mut s = Server::new();
        let src = "module t\n## Adds one.\nfn inc(x: u8) -> u8:\n  return x + 1\nfn g() -> u8:\n  return inc(2)\n";
        open(&mut s, URI, src);
        let pos = json!({"textDocument": {"uri": URI}, "position": {"line": 5, "character": 10}});
        let h = s.handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": pos}),
        );
        let text = h[0]["result"]["contents"]["value"].as_str().unwrap_or("");
        assert!(text.contains("fn inc(x: u8) -> u8:"), "{text}");
        assert!(text.contains("Adds one."), "{text}");
        let d = s.handle(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/definition", "params": pos}),
        );
        assert_eq!(d[0]["result"]["uri"], URI);
        assert_eq!(d[0]["result"]["range"]["start"]["line"], 2);
    }

    #[test]
    fn hover_shows_the_type_of_an_expression() {
        let mut s = Server::new();
        let src = "module t\nfn f(a: f32 mm) -> f32 mm:\n  return a + 1 mm\n";
        open(&mut s, URI, src);
        let pos = json!({"textDocument": {"uri": URI}, "position": {"line": 2, "character": 9}});
        let h = s.handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": pos}),
        );
        let text = h[0]["result"]["contents"]["value"].as_str().unwrap_or("");
        assert!(text.contains("f32 mm"), "{text}");
    }

    #[test]
    fn formatting_returns_one_edit() {
        let mut s = Server::new();
        open(&mut s, URI, "module t\nfn f( ):\n    pass\n");
        let r = s.handle(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/formatting",
            "params": {"textDocument": {"uri": URI}, "options": {"tabSize": 2, "insertSpaces": true}}
        }));
        assert_eq!(r[0]["result"][0]["newText"], "module t\nfn f():\n  pass\n");
    }

    #[test]
    fn unknown_requests_get_an_error_and_notifications_are_ignored() {
        let mut s = Server::new();
        let r = s.handle(
            &json!({"jsonrpc": "2.0", "id": 9, "method": "textDocument/rename", "params": {}}),
        );
        assert_eq!(r[0]["error"]["code"], -32601);
        assert!(s
            .handle(&json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {}}))
            .is_empty());
        s.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "shutdown"}));
        s.handle(&json!({"jsonrpc": "2.0", "method": "exit"}));
        assert!(s.was_shut_down() && s.exit_requested());
    }
}
