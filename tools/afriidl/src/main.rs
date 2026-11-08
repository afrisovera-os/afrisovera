//! AfriIDL v0 — the AfriSovera interface definition language compiler.
//!
//! Reads a `.afriidl` file and generates `no_std` Rust: one struct per `message`
//! (with fixed little-endian wire (de)serialization) and one method-id enum per
//! `protocol`. This is the toolchain piece that makes Mansa's "everything is
//! IPC" model maintainable — interfaces are declared once and code-generated,
//! giving a typed, versioned contract shared by services and clients.
//!
//! Scope (v0): scalar message fields only (u8/u16/u32/u64/i8/i16/i32/i64/bool);
//! methods record their signature and get stable ids. Strings, arrays, nested
//! messages, and full client/server stubs are future versions.
//!
//! Usage: `afriidl <input.afriidl> -o <output.rs>`

use std::process::exit;

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Colon,
    Comma,
    Semicolon,
    Arrow,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && i + 1 < b.len() && b[i + 1] == b'/' {
            // line comment
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < b.len() && b[i + 1] == b'*' {
            // block comment
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if c == '-' && i + 1 < b.len() && b[i + 1] == b'>' {
            out.push(Tok::Arrow);
            i += 2;
        } else if c == '{' {
            out.push(Tok::LBrace);
            i += 1;
        } else if c == '}' {
            out.push(Tok::RBrace);
            i += 1;
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if c == ':' {
            out.push(Tok::Colon);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if c == ';' {
            out.push(Tok::Semicolon);
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && {
                let ch = b[i] as char;
                ch.is_ascii_alphanumeric() || ch == '_'
            } {
                i += 1;
            }
            out.push(Tok::Ident(src[start..i].to_string()));
        } else {
            return Err(format!("unexpected character '{}'", c));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------
struct Field {
    name: String,
    ty: String,
}
struct Message {
    name: String,
    fields: Vec<Field>,
}
struct Method {
    name: String,
    params: Vec<Field>,
    ret: Option<String>,
}
struct Protocol {
    name: String,
    messages: Vec<Message>,
    methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------------------
struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn expect(&mut self, want: &Tok) -> Result<(), String> {
        match self.next() {
            Some(ref t) if t == want => Ok(()),
            other => Err(format!("expected {:?}, found {:?}", want, other)),
        }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            other => Err(format!("expected identifier, found {:?}", other)),
        }
    }

    fn parse_protocol(&mut self) -> Result<Protocol, String> {
        // `protocol <Name> { ... }`
        let kw = self.ident()?;
        if kw != "protocol" {
            return Err(format!("expected `protocol`, found `{}`", kw));
        }
        let name = self.ident()?;
        self.expect(&Tok::LBrace)?;
        let mut messages = Vec::new();
        let mut methods = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.next();
                    break;
                }
                Some(Tok::Ident(k)) if k == "message" => messages.push(self.parse_message()?),
                Some(Tok::Ident(k)) if k == "method" => methods.push(self.parse_method()?),
                other => return Err(format!("expected `message`, `method`, or `}}`, found {:?}", other)),
            }
        }
        Ok(Protocol { name, messages, methods })
    }

    fn parse_message(&mut self) -> Result<Message, String> {
        self.ident()?; // `message`
        let name = self.ident()?;
        self.expect(&Tok::LBrace)?;
        let mut fields = Vec::new();
        loop {
            if let Some(Tok::RBrace) = self.peek() {
                self.next();
                break;
            }
            let fname = self.ident()?;
            self.expect(&Tok::Colon)?;
            let ty = self.ident()?;
            fields.push(Field { name: fname, ty });
            // optional comma (trailing comma allowed)
            if let Some(Tok::Comma) = self.peek() {
                self.next();
            }
        }
        Ok(Message { name, fields })
    }

    fn parse_method(&mut self) -> Result<Method, String> {
        self.ident()?; // `method`
        let name = self.ident()?;
        self.expect(&Tok::LParen)?;
        let mut params = Vec::new();
        loop {
            if let Some(Tok::RParen) = self.peek() {
                self.next();
                break;
            }
            let pname = self.ident()?;
            self.expect(&Tok::Colon)?;
            let ty = self.ident()?;
            params.push(Field { name: pname, ty });
            if let Some(Tok::Comma) = self.peek() {
                self.next();
            }
        }
        let ret = if let Some(Tok::Arrow) = self.peek() {
            self.next();
            Some(self.ident()?)
        } else {
            None
        };
        self.expect(&Tok::Semicolon)?;
        Ok(Method { name, params, ret })
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------
fn scalar_size(ty: &str) -> Option<usize> {
    match ty {
        "u8" | "i8" | "bool" => Some(1),
        "u16" | "i16" => Some(2),
        "u32" | "i32" => Some(4),
        "u64" | "i64" => Some(8),
        _ => None,
    }
}

fn rust_type(ty: &str) -> &str {
    if ty == "bool" {
        "bool"
    } else {
        ty
    }
}

fn to_pascal(s: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------
fn generate(p: &Protocol, src_name: &str) -> Result<String, String> {
    let mut o = String::new();
    o.push_str(&format!(
        "// AUTO-GENERATED by AfriIDL v0 from `{}` — do not edit by hand.\n",
        src_name
    ));
    o.push_str("// Regenerate with: afriidl <input.afriidl> -o <output.rs>\n");
    o.push_str("#![allow(dead_code)]\n\n");

    for m in &p.messages {
        // Validate fields + compute layout.
        let mut offset = 0usize;
        let mut layout = Vec::new();
        for f in &m.fields {
            let sz = scalar_size(&f.ty).ok_or_else(|| {
                format!(
                    "message `{}` field `{}`: unsupported type `{}` (v0 supports scalars only)",
                    m.name, f.name, f.ty
                )
            })?;
            layout.push((f.name.clone(), f.ty.clone(), offset, sz));
            offset += sz;
        }
        let wire = offset;

        o.push_str(&format!("/// Wire message `{}` ({} bytes, little-endian).\n", m.name, wire));
        o.push_str("#[repr(C)]\n");
        o.push_str("#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]\n");
        o.push_str(&format!("pub struct {} {{\n", m.name));
        for (name, ty, _, _) in &layout {
            o.push_str(&format!("    pub {}: {},\n", name, rust_type(ty)));
        }
        o.push_str("}\n\n");

        o.push_str(&format!("impl {} {{\n", m.name));
        o.push_str(&format!("    pub const WIRE_SIZE: usize = {};\n\n", wire));

        // to_bytes
        o.push_str(&format!("    pub fn to_bytes(&self) -> [u8; {}] {{\n", wire));
        o.push_str(&format!("        let mut buf = [0u8; {}];\n", wire));
        for (name, ty, off, sz) in &layout {
            if ty == "bool" {
                o.push_str(&format!("        buf[{}] = self.{} as u8;\n", off, name));
            } else {
                o.push_str(&format!(
                    "        buf[{}..{}].copy_from_slice(&self.{}.to_le_bytes());\n",
                    off,
                    off + sz,
                    name
                ));
            }
        }
        o.push_str("        buf\n    }\n\n");

        // from_bytes
        o.push_str("    pub fn from_bytes(buf: &[u8]) -> Self {\n");
        o.push_str(&format!(
            "        assert!(buf.len() >= {}, \"buffer too small for {}\");\n",
            wire, m.name
        ));
        o.push_str("        Self {\n");
        for (name, ty, off, sz) in &layout {
            if ty == "bool" {
                o.push_str(&format!("            {}: buf[{}] != 0,\n", name, off));
            } else {
                o.push_str(&format!(
                    "            {}: {{ let mut t = [0u8; {}]; t.copy_from_slice(&buf[{}..{}]); {}::from_le_bytes(t) }},\n",
                    name,
                    sz,
                    off,
                    off + sz,
                    rust_type(ty),
                ));
            }
        }
        o.push_str("        }\n    }\n}\n\n");
    }

    // Method id enum for the protocol.
    if !p.methods.is_empty() {
        o.push_str(&format!(
            "/// Stable method ids for protocol `{}` (dispatch discriminants).\n",
            p.name
        ));
        o.push_str("#[repr(u32)]\n");
        o.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n");
        o.push_str(&format!("pub enum {}Method {{\n", to_pascal(&p.name)));
        for (i, m) in p.methods.iter().enumerate() {
            let ret = m.ret.clone().unwrap_or_else(|| "()".to_string());
            let params: Vec<String> =
                m.params.iter().map(|f| format!("{}: {}", f.name, f.ty)).collect();
            o.push_str(&format!(
                "    /// `{}({}) -> {}`\n",
                m.name,
                params.join(", "),
                ret
            ));
            o.push_str(&format!("    {} = {},\n", to_pascal(&m.name), i));
        }
        o.push_str("}\n");
    }

    Ok(o)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                output = Some(args.get(i).ok_or("`-o` needs a path")?.clone());
            }
            "-h" | "--help" => {
                println!("Usage: afriidl <input.afriidl> -o <output.rs>");
                return Ok(());
            }
            other => {
                if input.is_none() {
                    input = Some(other.to_string());
                } else {
                    return Err(format!("unexpected argument `{}`", other));
                }
            }
        }
        i += 1;
    }

    let input = input.ok_or("no input file (usage: afriidl <input.afriidl> -o <output.rs>)")?;
    let src = std::fs::read_to_string(&input).map_err(|e| format!("reading {}: {}", input, e))?;

    let toks = lex(&src)?;
    let mut parser = Parser { toks, pos: 0 };
    let proto = parser.parse_protocol()?;
    if parser.peek().is_some() {
        return Err("trailing tokens after protocol (v0 supports one protocol per file)".into());
    }
    let code = generate(&proto, &input)?;

    match output {
        Some(path) => {
            std::fs::write(&path, &code).map_err(|e| format!("writing {}: {}", path, e))?;
            eprintln!(
                "afriidl: {} → {} ({} message(s), {} method(s))",
                input,
                path,
                proto.messages.len(),
                proto.methods.len()
            );
        }
        None => print!("{}", code),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("afriidl: error: {}", e);
        exit(1);
    }
}
