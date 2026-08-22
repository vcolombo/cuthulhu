// SPDX-License-Identifier: GPL-3.0-or-later
//! The wiring half of the IPC seam: what the desktop registers, under the names JavaScript has to
//! use.
//!
//! Nothing else in the suite calls a real `#[tauri::command]` — `ui/e2e/smoke.spec.ts` installs a
//! fake over `invoke` — so a renamed command or argument used to ship green. It has: `trace_image`'s
//! Rust argument became `controls` while `ui/src/ipc.ts` kept sending `opts:`, and two commits cut
//! nothing while every check passed (#85).
//!
//! This test derives the inventory from the handler registry the app actually builds with and
//! compares it to the committed `ipc-inventory.json`, which the e2e fake then refuses to invoke
//! outside of. Each half is read in its own language: Rust parses Rust here, TypeScript checks
//! payloads there, and neither has to parse the other.
//!
//! Three rules decide what the file says, and all three are Tauri's rather than ours:
//!
//! - Only commands in `generate_handler!` exist. A function carrying the attribute but left out of
//!   the registry cannot be invoked, so listing it would tell the fake to accept a call that would
//!   fail against the real backend.
//! - Parameters Tauri fills in itself are not payload keys (`FRAMEWORK_PARAMS`).
//! - Argument names are lower camel by default, `rename_all` moves them, `rename` renames the
//!   command. `heck` does the conversion because the command macro uses `heck` (see
//!   `tauri-macros`' `wrapper.rs`), and a second implementation of "camelCase" is a second answer.
//!
//! Anything else — a pattern with no name to key on, an attribute option this does not know, a
//! registered command with no definition — fails the test. An omitted command is an unchecked
//! command, which is the failure this file exists to end.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToLowerCamelCase, ToSnakeCase};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{Attribute, Expr, ExprLit, File, FnArg, Item, ItemFn, Lit, Meta, Pat, Token, Type, UseTree};

/// Parameters Tauri supplies from the request instead of the payload: each implements `CommandArg`
/// by reaching into the message, so none of them is a key the frontend sends. Taken from the
/// `CommandArg` implementations in the pinned `tauri` crate, minus `Channel`, which *is* read from
/// the payload.
///
/// A name alone does not settle it: `custom::Request` is this crate's payload type and its key has
/// to be sent. A qualified path answers for itself, and a bare name is answered by the file's own
/// `use` items (see `Imports`), which is what `use tauri::State` and `use custom::Request` leave
/// indistinguishable in a signature. Dropping a real key would have the fake refuse a call the
/// backend accepts, which is this file's own failure mode rather than the one it is here to catch.
const FRAMEWORK_PARAMS: &[&str] = &[
    "AppHandle",
    "CommandScope",
    "GlobalScope",
    "Request",
    "State",
    "Webview",
    "WebviewWindow",
    "Window",
];

const INVENTORY_FILE: &str = "ipc-inventory.json";

/// Set to regenerate the committed inventory instead of failing on it.
const UPDATE_VAR: &str = "UPDATE_IPC_INVENTORY";

#[test]
fn the_committed_inventory_is_the_registered_command_surface() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = crate_dir.join("src");

    let registered = registered_commands(&src.join("main.rs"));
    assert!(!registered.is_empty(), "no commands registered in main.rs");
    let defined = command_definitions(&src);

    let mut inventory = BTreeMap::new();
    for function_name in &registered {
        let definition = defined.get(function_name).unwrap_or_else(|| {
            panic!(
                "`{function_name}` is registered with generate_handler! but no #[tauri::command] \
                 function of that name was found under {}",
                src.display()
            )
        });
        let command = external_command(&definition.function, &definition.imports)
            .unwrap_or_else(|e| panic!("cannot state the wire contract of `{function_name}`: {e}"));
        if let Some(other) = inventory.insert(command.name.clone(), command.args) {
            panic!(
                "two registered commands are invoked as `{}` (arguments {other:?}); \
                 one of them must be renamed",
                command.name
            );
        }
    }

    let expected = render(&inventory);
    let path = crate_dir.join(INVENTORY_FILE);
    let committed = fs::read_to_string(&path).unwrap_or_default();
    if committed == expected {
        return;
    }

    if std::env::var_os(UPDATE_VAR).is_some() {
        fs::write(&path, &expected).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        eprintln!("{INVENTORY_FILE} rewritten from the registered commands; commit it");
        return;
    }

    let only_in_source = lines_missing_from(&expected, &committed);
    let only_in_file = lines_missing_from(&committed, &expected);
    panic!(
        "apps/desktop/{INVENTORY_FILE} does not match the registered commands.\n\
         only in the source: {only_in_source}\n\
         only in the committed file: {only_in_file}\n\
         regenerate with `{UPDATE_VAR}=1 cargo test -p desktop --test ipc_inventory` and commit it.\n\
         The e2e fake refuses whatever this file does not declare, so a stale copy is a call the \
         frontend can still make and the backend will still reject."
    );
}

/// What one command looks like from JavaScript.
struct ExternalCommand {
    name: String,
    args: Vec<String>,
}

/// The externally invoked names of everything in `generate_handler!`, by the last segment of each
/// registered path (`ipc::new_doc` → `new_doc`), in registration order.
fn registered_commands(main_rs: &Path) -> Vec<String> {
    let mut found = FindHandlerRegistry::default();
    found.visit_file(&parse(main_rs));
    match found.registries.len() {
        1 => found.registries.remove(0),
        0 => panic!("no generate_handler! in {}", main_rs.display()),
        // Which one the built app uses is not something reading the source can settle, and the
        // inventory would silently describe whichever came first.
        n => panic!("{n} generate_handler! registries in {}", main_rs.display()),
    }
}

#[derive(Default)]
struct FindHandlerRegistry {
    registries: Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for FindHandlerRegistry {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.segments.last().is_some_and(|s| s.ident == "generate_handler") {
            let paths = mac
                .parse_body_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                .expect("generate_handler! takes a comma-separated list of command paths");
            self.registries.push(
                paths
                    .iter()
                    .map(|p| {
                        p.segments
                            .last()
                            .expect("a command path has at least one segment")
                            .ident
                            .to_string()
                    })
                    .collect(),
            );
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Every `#[tauri::command]` function under `src`, by function name.
fn command_definitions(src: &Path) -> BTreeMap<String, Definition> {
    let mut defined = BTreeMap::new();
    for file in rust_files(src) {
        collect_commands(&parse(&file).items, &file, &Imports::default(), &mut defined);
    }
    defined
}

/// A command function together with what the names in its signature mean.
struct Definition {
    function: ItemFn,
    imports: Imports,
}

/// Walks one module's items with that module's own imports. Scopes are exact rather than merged,
/// because Rust does not lend a `use` to a nested module: a command inside `mod inner` sees
/// `inner`'s imports, and `use super::Window` is how it reaches for its parent's — which is why the
/// parent's scope is passed in rather than dropped.
fn collect_commands(items: &[Item], file: &Path, parent: &Imports, into: &mut BTreeMap<String, Definition>) {
    // Collected first, in a pass of its own: a `use` is in scope for the whole module regardless of
    // whether it is written above or below the function that reads it.
    let mut written = Imports::default();
    for item in items {
        if let Item::Use(u) = item {
            collect_use_tree(&u.tree, &mut Vec::new(), &mut written);
        }
    }
    let imports = written.resolved_against(parent);

    for item in items {
        match item {
            Item::Fn(f) if f.attrs.iter().any(is_command_attribute) => {
                let name = f.sig.ident.to_string();
                let definition = Definition { function: f.clone(), imports: imports.clone() };
                if into.insert(name.clone(), definition).is_some() {
                    // Not merely ambiguous here: `generate_handler!` builds a match on the bare
                    // function name, so two commands sharing one would be two identical arms and
                    // only the first could ever be reached (`tauri-macros`' `handler.rs`).
                    panic!("two #[tauri::command] functions are named `{name}` ({})", file.display());
                }
            }
            // Commands inside an inline module are as invocable as any other; missing them would
            // leave the registry naming something this file could not find.
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    collect_commands(items, file, &imports, into);
                }
            }
            _ => {}
        }
    }
}

/// What a name in a signature refers to: the `use` items of the module the command is written in.
///
/// One local name can still stand for several paths — `use a::X; use b::X as X;` does not compile,
/// but `use a::X;` twice does, and a rename can land a second path on the same name. The paths are
/// all kept, and the caller refuses only where they disagree about being Tauri's, because either
/// wrong answer drops or invents a payload key.
#[derive(Clone, Default)]
struct Imports {
    /// Local name to every path this module imports it from.
    names: BTreeMap<String, Vec<Vec<String>>>,
    /// A glob can bring in a name this cannot see, so an unresolved name is not evidence of a
    /// local type once one is present.
    globs: bool,
}

impl Imports {
    fn add(&mut self, local: String, path: Vec<String>) {
        let paths = self.names.entry(local).or_default();
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    /// The same imports with `use super::X` replaced by whatever `X` is in the enclosing module —
    /// which is how a nested module reaches an import it cannot inherit. Left as written, the
    /// parameter would read as this crate's own and its key would be sent for something Tauri
    /// fills in.
    ///
    /// `use super::super::X` and `use super::x::Y` are left alone: the first needs a scope this
    /// does not hold, and the second names an item rather than an import. Both then read as a
    /// payload key, which is the harmless direction — a declared key nobody sends, rather than a
    /// sent key nothing declares.
    fn resolved_against(&self, parent: &Imports) -> Imports {
        let mut resolved = Imports { names: BTreeMap::new(), globs: self.globs };
        for (local, paths) in &self.names {
            for path in paths {
                match path.as_slice() {
                    [first, name] if first == "super" => match parent.names.get(name) {
                        Some(inherited) => {
                            for path in inherited {
                                resolved.add(local.clone(), path.clone());
                            }
                        }
                        None => resolved.add(local.clone(), path.clone()),
                    },
                    _ => resolved.add(local.clone(), path.clone()),
                }
            }
        }
        resolved
    }
}

fn collect_use_tree(tree: &UseTree, prefix: &mut Vec<String>, into: &mut Imports) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_use_tree(&p.tree, prefix, into);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut path = prefix.clone();
            path.push(n.ident.to_string());
            into.add(n.ident.to_string(), path);
        }
        UseTree::Rename(r) => {
            let mut path = prefix.clone();
            path.push(r.ident.to_string());
            into.add(r.rename.to_string(), path);
        }
        UseTree::Glob(_) => into.globs = true,
        UseTree::Group(g) => {
            for item in &g.items {
                collect_use_tree(item, prefix, into);
            }
        }
    }
}

fn is_command_attribute(attr: &Attribute) -> bool {
    let path = attr.path();
    path.is_ident("command")
        || (path.segments.len() == 2
            && path.segments[0].ident == "tauri"
            && path.segments[1].ident == "command")
}

/// Which spelling the command macro gives arguments.
#[derive(Clone, Copy)]
enum ArgumentCase {
    Camel,
    Snake,
}

/// `#[tauri::command]`'s own options: `async` is a bare keyword rather than a `Meta`, which is why
/// this cannot just parse a meta list.
enum CommandOption {
    Async,
    Meta(Meta),
}

impl Parse for CommandOption {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![async]) {
            input.parse::<Token![async]>()?;
            return Ok(Self::Async);
        }
        Ok(Self::Meta(input.parse()?))
    }
}

fn external_command(function: &ItemFn, imports: &Imports) -> Result<ExternalCommand, String> {
    let attr = function
        .attrs
        .iter()
        .find(|a| is_command_attribute(a))
        .ok_or("no #[tauri::command] attribute")?;

    let mut case = ArgumentCase::Camel;
    let mut renamed = None;
    if !matches!(attr.meta, Meta::Path(_)) {
        let options = attr
            .parse_args_with(Punctuated::<CommandOption, Token![,]>::parse_terminated)
            .map_err(|e| format!("cannot read the attribute's options: {e}"))?;
        for option in options {
            let meta = match option {
                CommandOption::Async => continue,
                CommandOption::Meta(meta) => meta,
            };
            let Meta::NameValue(nv) = meta else {
                return Err("an attribute option this does not know how to read".into());
            };
            let value = match &nv.value {
                Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => s.value(),
                _ => return Err("an attribute option whose value is not a string".into()),
            };
            if nv.path.is_ident("rename_all") {
                case = match value.as_str() {
                    "camelCase" => ArgumentCase::Camel,
                    "snake_case" => ArgumentCase::Snake,
                    other => return Err(format!("rename_all = \"{other}\"")),
                };
            } else if nv.path.is_ident("rename") {
                renamed = Some(value);
            } else if !nv.path.is_ident("root") {
                // `root` moves where the macro finds `tauri`, which no name depends on. Anything
                // else may well move a name, and guessing which way is how this file would start
                // describing a surface that is not there.
                return Err(format!(
                    "the attribute option `{}` is not one this knows the naming effect of",
                    quote_path(&nv.path)
                ));
            }
        }
    }

    let ident = function.sig.ident.to_string();
    let name = match renamed {
        Some(name) => name,
        // The macro stringifies the identifier verbatim, raw prefix and all. Rather than encode
        // that, refuse: a command named after a keyword is a rename away from being sayable.
        None if ident.starts_with("r#") => return Err(format!("a raw identifier (`{ident}`)")),
        None => ident,
    };

    let mut args = Vec::new();
    for arg in &function.sig.inputs {
        let FnArg::Typed(typed) = arg else {
            return Err("a `self` parameter".into());
        };
        if is_framework_param(&typed.ty, imports)? {
            continue;
        }
        let Pat::Ident(pat) = &*typed.pat else {
            // The macro keys wildcard and destructured parameters off something other than a name
            // (the empty string, the type's identifier). None is a payload key worth mirroring by
            // hand, and none appears at this seam.
            return Err("a parameter with no name to key on".into());
        };
        // `unraw`, as the macro does: `r#type` is sent as `type`.
        let key = pat.ident.to_string();
        let key = key.strip_prefix("r#").unwrap_or(&key);
        let key = match case {
            ArgumentCase::Camel => key.to_lower_camel_case(),
            ArgumentCase::Snake => key.to_snake_case(),
        };
        if args.contains(&key) {
            return Err(format!("two parameters that both arrive as `{key}`"));
        }
        args.push(key);
    }
    // Sorted, not in declaration order: the payload is a JSON object, so reordering a Rust
    // signature changes nothing the frontend can see and should not show up as a change here.
    args.sort();

    Ok(ExternalCommand { name, args })
}

/// Whether Tauri fills this parameter in from the request. `Err` when the spelling cannot say.
fn is_framework_param(ty: &Type, imports: &Imports) -> Result<bool, String> {
    let Type::Path(path) = ty else { return Ok(false) };
    let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
    let Some((first, rest)) = segments.split_first() else { return Ok(false) };

    // The leading segment is a local name before it is anything else, and an import can rename
    // either the type (`use tauri::Window as W`) or the module it came from (`use tauri as t;
    // t::Window`). Resolving it first is what keeps a rename from hiding what the parameter is:
    // reading the written name would make `W` a payload key Tauri never lets the frontend send.
    let imported = imports.names.get(first);
    let spellings: Vec<Vec<String>> = match imported {
        Some(paths) => paths.iter().map(|path| [path.as_slice(), rest].concat()).collect(),
        None => vec![segments.clone()],
    };

    let answers: BTreeSet<bool> = spellings.iter().map(|path| names_a_framework_param(path)).collect();
    if answers.len() > 1 {
        // Two imports of this name disagree about it, and either wrong answer writes a wrong
        // inventory: Tauri's own read as a payload declares a key nobody sends, and a payload type
        // read as Tauri's drops a key that is sent, so the fake refuses a call the backend accepts.
        return Err(format!("`{first}`, which this file imports under two paths that disagree"));
    }
    let framework = answers.into_iter().next().unwrap_or(false);

    // Named by no import, so a type of this crate's own — none of these names is in the prelude. A
    // glob is the one thing that can hide Tauri's own here, and only under its real name, since a
    // glob cannot rename what it brings in.
    if !framework && imported.is_none() && imports.globs && rest.is_empty() && FRAMEWORK_PARAMS.contains(&first.as_str()) {
        return Err(format!(
            "`{first}` alongside a glob import, so whether it is Tauri's cannot be read here — \
             spell Tauri's as `tauri::{first}`"
        ));
    }
    Ok(framework)
}

/// Whether a path, spelled out in full, names one of the parameters Tauri fills in itself.
fn names_a_framework_param(path: &[String]) -> bool {
    path.first().is_some_and(|first| first == "tauri")
        && path.last().is_some_and(|name| FRAMEWORK_PARAMS.contains(&name.as_str()))
}

fn quote_path(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// One line per command, so a pull request shows which command changed rather than a reflowed
/// block. JSON because the e2e fake imports it directly.
fn render(inventory: &BTreeMap<String, Vec<String>>) -> String {
    let mut out = String::from("{\n");
    for (i, (name, args)) in inventory.iter().enumerate() {
        let keys: Vec<String> = args.iter().map(|a| json_string(a)).collect();
        let comma = if i + 1 == inventory.len() { "" } else { "," };
        out.push_str(&format!("  {}: [{}]{comma}\n", json_string(name), keys.join(", ")));
    }
    out.push_str("}\n");
    out
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

fn lines_missing_from(these: &str, those: &str) -> String {
    let missing: Vec<&str> = these
        .lines()
        .filter(|l| !those.lines().any(|other| other == *l))
        .map(str::trim)
        .collect();
    if missing.is_empty() {
        "nothing".into()
    } else {
        missing.join(" ")
    }
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn parse(file: &Path) -> File {
    let source = fs::read_to_string(file).unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
    syn::parse_file(&source).unwrap_or_else(|e| panic!("cannot parse {}: {e}", file.display()))
}

mod naming {
    use super::*;

    /// Parsed as a file rather than a lone function, because a bare type name in the signature is
    /// answered by the `use` items of the module around it — so the helper takes the same route the
    /// generator does, `collect_commands`, and reads back what it recorded for the command.
    fn command(source: &str) -> Result<ExternalCommand, String> {
        let file = syn::parse_file(source).expect("a file");
        let mut defined = BTreeMap::new();
        collect_commands(&file.items, Path::new("<test>"), &Imports::default(), &mut defined);
        let (_, definition) = defined.iter().next().expect("a command function");
        external_command(&definition.function, &definition.imports)
    }

    #[test]
    fn arguments_arrive_in_lower_camel_and_framework_parameters_do_not_arrive_at_all() {
        let c = command(
            "#[tauri::command]
             fn travel_for_order(state: tauri::State<AppStateHandle>, doc_revision: String) {}",
        )
        .expect("a supported shape");
        assert_eq!(c.name, "travel_for_order");
        assert_eq!(c.args, ["docRevision"]);
    }

    #[test]
    fn rename_all_moves_the_arguments_and_rename_moves_the_command() {
        let c = command("#[tauri::command(rename_all = \"snake_case\")] fn f(doc_revision: String) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["doc_revision"]);

        let c = command("#[tauri::command(async, rename = \"cutIt\")] fn cut(request: R) {}")
            .expect("a supported shape");
        assert_eq!(c.name, "cutIt");
        assert_eq!(c.args, ["request"]);
    }

    #[test]
    fn async_is_not_an_argument_and_neither_is_the_app_handle() {
        let c = command("#[tauri::command(async)] fn force_quit(app: tauri::AppHandle) {}")
            .expect("a supported shape");
        assert!(c.args.is_empty(), "{:?}", c.args);
    }

    /// A key dropped here is a call the fake refuses and the backend would have accepted, so the
    /// framework's own types have to be told apart from a payload type that shares a name — which
    /// for a bare name is a question about the file's imports, not about the signature.
    #[test]
    fn a_payload_type_named_like_a_framework_one_still_sends_its_key() {
        let c = command("#[tauri::command] fn f(request: custom::Request, state: tauri::State<S>) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["request"]);

        // The import decides, both ways round.
        let c = command("use tauri::Window; #[tauri::command] fn f(w: Window, id: String) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["id"]);

        let c = command("use custom::Request; #[tauri::command] fn f(request: Request, id: String) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["id", "request"]);

        // Not imported at all: this crate's own, since none of these names is in the prelude.
        let c = command("#[tauri::command] fn f(window: Window) {}").expect("a supported shape");
        assert_eq!(c.args, ["window"]);

        // A rename moves the local name, not the type behind it — neither of these sends a key.
        let c = command("use tauri::Window as W; #[tauri::command] fn f(w: W, id: String) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["id"]);

        let c = command("use tauri as t; #[tauri::command] fn f(w: t::Window, id: String) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["id"]);

        // The same rename over a payload type keeps its key, which is the half a name-first reading
        // got right and an import-blind one gets wrong in the other direction.
        let c = command("use custom::Window as W; #[tauri::command] fn f(w: W) {}")
            .expect("a supported shape");
        assert_eq!(c.args, ["w"]);

        // Each module answers for itself. An unrelated `mod inner` shadowing the name locally says
        // nothing about a command outside it, and Rust does not lend the outer `use` inward either.
        let c = command(
            "use tauri::Window; mod inner { use custom::Window; }
             #[tauri::command] fn f(w: Window, id: String) {}",
        )
        .expect("a supported shape");
        assert_eq!(c.args, ["id"]);

        let c = command(
            "use tauri::Window;
             mod inner { use custom::Window; #[tauri::command] pub fn f(w: Window) {} }",
        )
        .expect("a supported shape");
        assert_eq!(c.args, ["w"]);

        // …and `use super::Window` is how it reaches the import it cannot inherit, so that one
        // resolves to the parent's, not to a type of this crate's own.
        let c = command(
            "use tauri::Window;
             mod inner { use super::Window; #[tauri::command] pub fn f(w: Window, id: String) {} }",
        )
        .expect("a supported shape");
        assert_eq!(c.args, ["id"]);

        // A glob can bring Tauri's own in under its real name, and nothing here sees through one.
        assert!(command("use tauri::*; #[tauri::command] fn f(w: Window) {}").is_err());

        // Two imports the compiler keeps apart with `cfg` both land in the same scope here, and a
        // signature that means one thing on Unix and another on Windows has no single answer.
        assert!(command(
            "#[cfg(unix)] use tauri::Window; #[cfg(windows)] use custom::Window;
             #[tauri::command] fn f(w: Window) {}",
        )
        .is_err());
    }

    /// The whole point of failing here: an inventory that quietly omitted this command would tell
    /// the e2e fake to accept a payload the real backend does not.
    #[test]
    fn a_shape_with_no_key_to_send_is_refused_rather_than_omitted() {
        assert!(command("#[tauri::command] fn f(_: String) {}").is_err());
        assert!(command("#[tauri::command(rename_all = \"kebab-case\")] fn f(a: String) {}").is_err());
        assert!(command("#[tauri::command(unknown = \"x\")] fn f() {}").is_err());
    }
}
