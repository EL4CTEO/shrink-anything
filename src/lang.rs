//! The language table. Everything shrink-anything knows about a language is data in here:
//! how comments and strings look, how a block ends, what a definition looks like, and which
//! names a runtime or framework calls without the code ever mentioning them.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// How a definition's body ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    /// Braces (or a statement that ends where the brackets balance).
    Brace,
    /// Indentation.
    Indent,
    /// An end keyword at the indentation of the opening line.
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Function,
    Method,
    Type,
    Const,
    /// Belongs to another name and goes when that name goes: an impl block, a prototype, an extension.
    Attached,
}

/// How a member says "nobody outside can call me".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Private {
    /// This word on the definition line.
    Keyword(&'static str),
    /// A leading underscore.
    Underscore,
    /// A lowercase first letter (Go).
    Lowercase,
    /// No pub (Rust).
    NotPub,
    Never,
}

pub struct Spec {
    pub name: &'static str,
    pub exts: &'static [&'static str],
    /// Whole file names, for files without an extension.
    pub names: &'static [&'static str],
    pub line_comments: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    pub quotes: &'static str,
    /// A plain quoted string may run over several lines.
    pub multiline: bool,
    /// Triple-quoted strings.
    pub triple: bool,
    pub block: Block,
    /// Patterns for a definition line, matched after comments and strings are blanked. The group "name" is the name.
    pub defs: &'static [(&'static str, Kind)],
    /// Names called by the runtime or a framework.
    pub keep: &'static [&'static str],
    pub keep_prefixes: &'static [&'static str],
    pub private: Private,
    /// On a type's line: it has a base that may register or call it.
    pub inherits: &'static [&'static str],
    /// Files are imported by name, so a file nothing names is dead.
    pub file_refs: bool,
    /// A file with one of these looks names up at run time; everything in it needs a human or agent.
    pub dynamic: &'static [&'static str],
    /// A command that parses one file (the path is appended). The first word may list alternatives with |.
    pub syntax: &'static [&'static str],
}

pub struct Lang {
    pub spec: &'static Spec,
    pub defs: Vec<(Regex, Kind)>,
}

use Kind::{Attached, Const, Function, Method, Type};

const BASE: Spec = Spec {
    name: "",
    exts: &[],
    names: &[],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: "\"'",
    multiline: false,
    triple: false,
    block: Block::Brace,
    defs: &[],
    keep: &[],
    keep_prefixes: &[],
    private: Private::Never,
    inherits: &[],
    file_refs: false,
    dynamic: &[],
    syntax: &[],
};

const JS_FUNCTION: &str = r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*(?P<name>[A-Za-z_]\w*)";
const JS_ARROW: &str = r"^\s*(?:export\s+)?(?:const|let|var)\s+(?P<name>[A-Za-z_]\w*)\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\b|(?:\([^)]*\)|[A-Za-z_]\w*)\s*(?::\s*[^=]+?)?\s*=>)";
const JS_CONST: &str = r"^(?:export\s+)?const\s+(?P<name>[A-Za-z_]\w*)\s*(?::[^=]+)?=[^=>]";
const JS_CLASS: &str = r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+(?P<name>[A-Za-z_]\w*)";
const JS_METHOD: &str = r"^\s+(?:(?:public|private|protected|static|async|readonly|get|set)\s+)*(?P<name>#?[A-Za-z_]\w*)\s*(?:<[^>(]*>)?\([^)]*\)\s*(?::\s*[^{;=]+)?\{\s*$";
const TS_TYPE: &str =
    r"^\s*(?:export\s+)?(?:declare\s+)?(?:const\s+)?(?:interface|enum|type|namespace)\s+(?P<name>[A-Za-z_]\w*)";
const JS_KEEP: &[&str] = &[
    "constructor",
    "render",
    "toString",
    "toJSON",
    "valueOf",
    "setup",
    "data",
    "mounted",
    "created",
    "computed",
    "componentDidMount",
    "componentDidUpdate",
    "componentWillUnmount",
    "getDerivedStateFromProps",
    "connectedCallback",
    "disconnectedCallback",
    "attributeChangedCallback",
    "ngOnInit",
    "ngOnDestroy",
    "handler",
    "middleware",
    "loader",
    "action",
    "generateMetadata",
    "generateStaticParams",
    "getServerSideProps",
    "getStaticProps",
    "getStaticPaths",
];
const JS_DYNAMIC: &[&str] = &["eval(", "new Function("];
const JAVA_KEEP: &[&str] = &[
    "main",
    "toString",
    "equals",
    "hashCode",
    "compareTo",
    "run",
    "call",
    "close",
    "accept",
    "apply",
    "iterator",
    "next",
    "hasNext",
    "finalize",
    "clone",
    "readObject",
    "writeObject",
    "readResolve",
    "valueOf",
    "values",
];
const DOTNET_KEEP: &[&str] = &[
    "Main",
    "ToString",
    "Equals",
    "GetHashCode",
    "Dispose",
    "DisposeAsync",
    "CompareTo",
    "GetEnumerator",
    "Configure",
    "ConfigureServices",
    "Invoke",
    "InvokeAsync",
    "Awake",
    "Start",
    "Update",
    "FixedUpdate",
    "LateUpdate",
    "Reset",
];
const C_FUNCTION: &str = r"^(?:(?:static|inline|extern|const|unsigned|signed|struct|constexpr|virtual|explicit|friend|enum|union|long|short)\s+)*[A-Za-z_][\w:<>]*(?:\s*[*&]+\s*|\s+)(?P<name>[A-Za-z_]\w*)\s*\([^;]*$";
const C_PROTOTYPE: &str = r"^(?:(?:static|inline|extern|const|unsigned|signed|struct|constexpr|enum|union|long|short)\s+)*[A-Za-z_][\w:<>]*(?:\s*[*&]+\s*|\s+)(?P<name>[A-Za-z_]\w*)\s*\([^{}]*\)\s*;\s*$";
const LUA_DEFS: &[(&str, Kind)] = &[
    (r"^\s*(?:local\s+)?function\s+(?:[\w.]+[.:])?(?P<name>[A-Za-z_]\w*)\s*\(", Function),
    (r"^\s*(?:local\s+)?(?:[\w.]+\.)?(?P<name>[A-Za-z_]\w*)\s*=\s*function\s*\(", Function),
];
const LUA_KEEP: &[&str] = &["Init", "Start", "KnitInit", "KnitStart", "new", "Destroy", "main"];
const LUA_DYNAMIC: &[&str] = &["_G[", "getfenv", "loadstring"];

pub static SPECS: &[Spec] = &[
    Spec {
        name: "Python",
        exts: &["py", "pyi"],
        line_comments: &["#"],
        block_comment: None,
        triple: true,
        block: Block::Indent,
        defs: &[
            (r"^\s*(?:async\s+)?def\s+(?P<name>[A-Za-z_]\w*)\s*\(", Function),
            (r"^\s*class\s+(?P<name>[A-Za-z_]\w*)", Type),
            (r"^(?P<name>[A-Z][A-Z0-9_]{2,})\s*(?::[^=]+)?=[^=]", Const),
        ],
        keep: &["main", "setUp", "tearDown", "setUpClass", "tearDownClass", "setup_module", "teardown_module"],
        keep_prefixes: &["pytest_", "visit_", "do_", "on_", "handle_"],
        private: Private::Underscore,
        inherits: &["("],
        file_refs: true,
        dynamic: &["globals()", "locals()", "eval(", "exec(", "fire.Fire", "getmembers", "__getattr__"],
        syntax: &["python|python3|py", "-c", "import ast,sys;ast.parse(open(sys.argv[1],encoding='utf-8').read())"],
        ..BASE
    },
    Spec {
        name: "JavaScript",
        exts: &["js", "jsx", "mjs", "cjs"],
        quotes: "\"'\x60",
        defs: &[
            (JS_FUNCTION, Function),
            (JS_ARROW, Function),
            (JS_CONST, Const),
            (JS_CLASS, Type),
            (JS_METHOD, Method),
        ],
        keep: JS_KEEP,
        private: Private::Keyword("private"),
        inherits: &[" extends "],
        file_refs: true,
        dynamic: JS_DYNAMIC,
        syntax: &["node", "--check"],
        ..BASE
    },
    Spec {
        name: "TypeScript",
        exts: &["ts", "tsx", "mts", "cts"],
        quotes: "\"'\x60",
        defs: &[
            (JS_FUNCTION, Function),
            (JS_ARROW, Function),
            (JS_CONST, Const),
            (JS_CLASS, Type),
            (TS_TYPE, Type),
            (JS_METHOD, Method),
        ],
        keep: JS_KEEP,
        private: Private::Keyword("private"),
        inherits: &[" extends ", " implements "],
        file_refs: true,
        dynamic: JS_DYNAMIC,
        ..BASE
    },
    Spec {
        name: "Rust",
        exts: &["rs"],
        multiline: true,
        defs: &[
            (
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+)?fn\s+(?P<name>[A-Za-z_]\w*)",
                Function,
            ),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait|union|type)\s+(?P<name>[A-Za-z_]\w*)", Type),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+(?:mut\s+)?(?P<name>[A-Z_][A-Z0-9_]*)\s*:", Const),
            (
                r"^\s*(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[\w:<>,' ]+\s+for\s+)?(?:[\w:]+::)?(?P<name>[A-Za-z_]\w*)",
                Attached,
            ),
            (r"^\s*macro_rules!\s+(?P<name>[A-Za-z_]\w*)", Function),
        ],
        keep: &["main"],
        private: Private::NotPub,
        ..BASE
    },
    Spec {
        name: "Go",
        exts: &["go"],
        quotes: "\"'\x60",
        defs: &[
            (r"^func\s+(?P<name>[A-Za-z_]\w*)\s*[(\[]", Function),
            (r"^func\s+\([^)]*\)\s+(?P<name>[A-Za-z_]\w*)\s*[(\[]", Method),
            (r"^func\s+\(\s*\w*\s*\*?\s*(?P<name>[A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\)", Attached),
            (r"^type\s+(?P<name>[A-Za-z_]\w*)\s", Type),
            (r"^(?:const|var)\s+(?P<name>[A-Za-z_]\w*)\b[^(]*$", Const),
        ],
        keep: &["main", "init", "String", "Error", "MarshalJSON", "UnmarshalJSON", "ServeHTTP", "Len", "Less", "Swap"],
        keep_prefixes: &["Test", "Benchmark", "Example", "Fuzz"],
        private: Private::Lowercase,
        ..BASE
    },
    Spec {
        name: "Java",
        exts: &["java"],
        triple: true,
        defs: &[
            (
                r"^\s*(?:(?:public|private|protected|static|final|abstract|sealed|non-sealed|strictfp)\s+)*(?:class|interface|enum|record)\s+(?P<name>[A-Za-z_]\w*)",
                Type,
            ),
            (
                r"^\s+(?:(?:public|private|protected|static|final|synchronized|default)\s+)+(?:<[^>]+>\s+)?[\w<>\[\],.? ]*?[\w>\]]\s+(?P<name>[A-Za-z_]\w*)\s*\([^;]*$",
                Method,
            ),
        ],
        keep: JAVA_KEEP,
        private: Private::Keyword("private"),
        inherits: &[" extends ", " implements "],
        ..BASE
    },
    Spec {
        name: "Kotlin",
        exts: &["kt", "kts"],
        triple: true,
        defs: &[
            (
                r"^\s*(?:(?:public|private|protected|internal|open|abstract|final|suspend|inline|operator|infix|tailrec|external|actual|expect)\s+)*fun\s+(?:<[^>]+>\s+)?(?:[\w<>?,.]+\.)?(?P<name>[A-Za-z_]\w*)\s*\(",
                Function,
            ),
            (
                r"^\s*(?:(?:public|private|protected|internal|open|abstract|final|data|sealed|enum|annotation|inner|value|actual|expect)\s+)*(?:class|interface|object)\s+(?P<name>[A-Za-z_]\w*)",
                Type,
            ),
        ],
        keep: JAVA_KEEP,
        private: Private::Keyword("private"),
        inherits: &[":"],
        ..BASE
    },
    Spec {
        name: "C#",
        exts: &["cs"],
        triple: true,
        defs: &[
            (
                r"^\s*(?:(?:public|private|protected|internal|static|abstract|sealed|partial|readonly|unsafe)\s+)*(?:class|interface|struct|enum|record)\s+(?P<name>[A-Za-z_]\w*)",
                Type,
            ),
            (
                r"^\s+(?:(?:public|private|protected|internal|static|virtual|sealed|async|partial|readonly|extern|unsafe|new)\s+)+[\w<>\[\],.? ]*?[\w>\]?]\s+(?P<name>[A-Za-z_]\w*)\s*(?:<[^>]*>)?\([^;]*$",
                Method,
            ),
        ],
        keep: DOTNET_KEEP,
        keep_prefixes: &["On"],
        private: Private::Keyword("private"),
        inherits: &[":"],
        ..BASE
    },
    Spec {
        name: "C",
        exts: &["c", "h"],
        defs: &[(C_PROTOTYPE, Attached), (C_FUNCTION, Function)],
        keep: &["main"],
        private: Private::Keyword("static"),
        ..BASE
    },
    Spec {
        name: "C++",
        exts: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        defs: &[(C_PROTOTYPE, Attached), (C_FUNCTION, Function)],
        keep: &["main"],
        private: Private::Keyword("static"),
        ..BASE
    },
    Spec {
        name: "PHP",
        exts: &["php"],
        line_comments: &["//", "#"],
        multiline: true,
        defs: &[
            (
                r"^\s*(?:(?:public|private|protected|static|final|abstract)\s+)*function\s+&?(?P<name>[A-Za-z_]\w*)\s*\(",
                Function,
            ),
            (r"^\s*(?:(?:final|abstract|readonly)\s+)*(?:class|interface|trait|enum)\s+(?P<name>[A-Za-z_]\w*)", Type),
        ],
        keep: &["main", "handle", "boot", "register", "up", "down", "run"],
        private: Private::Keyword("private"),
        inherits: &[" extends ", " implements "],
        file_refs: true,
        dynamic: &["call_user_func", "$$"],
        syntax: &["php", "-l"],
        ..BASE
    },
    Spec {
        name: "Ruby",
        exts: &["rb", "rake", "gemspec"],
        names: &["Rakefile", "Gemfile"],
        line_comments: &["#"],
        block_comment: None,
        multiline: true,
        block: Block::End,
        defs: &[
            (r"^\s*def\s+(?:self\.)?(?P<name>[A-Za-z_]\w*)", Function),
            (r"^\s*(?:class|module)\s+(?P<name>[A-Z]\w*)", Type),
        ],
        keep: &[
            "initialize",
            "to_s",
            "inspect",
            "each",
            "call",
            "method_missing",
            "respond_to_missing",
            "index",
            "show",
            "new",
            "create",
            "edit",
            "update",
            "destroy",
            "perform",
            "up",
            "down",
            "change",
        ],
        inherits: &["<"],
        file_refs: true,
        syntax: &["ruby", "-c"],
        ..BASE
    },
    Spec {
        name: "Swift",
        exts: &["swift"],
        triple: true,
        defs: &[
            (
                r"^\s*(?:(?:public|private|fileprivate|internal|open|static|class|final|mutating|nonmutating|convenience)\s+)*func\s+(?P<name>[A-Za-z_]\w*)",
                Function,
            ),
            (
                r"^\s*(?:(?:public|private|fileprivate|internal|open|final|indirect)\s+)*(?:class|struct|enum|protocol|actor)\s+(?P<name>[A-Za-z_]\w*)",
                Type,
            ),
            (r"^\s*(?:(?:public|private|fileprivate|internal)\s+)*extension\s+(?P<name>[A-Za-z_]\w*)", Attached),
        ],
        keep: &["main", "body", "hash", "encode", "makeBody", "application", "scene"],
        private: Private::Keyword("private"),
        inherits: &[":"],
        ..BASE
    },
    Spec {
        name: "Dart",
        exts: &["dart"],
        triple: true,
        defs: &[
            (
                r"^\s*(?:(?:abstract|sealed|final|base)\s+)*(?:class|mixin|enum|extension)\s+(?P<name>[A-Za-z_]\w*)",
                Type,
            ),
            (r"^(?:[\w<>?,\[\]]+\s+)+(?P<name>[A-Za-z_]\w*)\s*\([^;]*$", Function),
            (r"^\s+(?:static\s+)?(?:[\w<>?,\[\]]+\s+)+(?P<name>_\w+)\s*\([^;]*$", Method),
        ],
        keep: &["main", "build", "createState", "initState", "dispose", "toString"],
        private: Private::Underscore,
        inherits: &[" extends ", " with ", " implements "],
        file_refs: true,
        ..BASE
    },
    Spec {
        name: "Lua",
        exts: &["lua"],
        line_comments: &["--"],
        block_comment: None,
        block: Block::End,
        defs: LUA_DEFS,
        keep: LUA_KEEP,
        private: Private::Keyword("local"),
        file_refs: true,
        dynamic: LUA_DYNAMIC,
        syntax: &["luac", "-p"],
        ..BASE
    },
    Spec {
        name: "Luau",
        exts: &["luau"],
        line_comments: &["--"],
        block_comment: None,
        quotes: "\"'\x60",
        block: Block::End,
        defs: LUA_DEFS,
        keep: LUA_KEEP,
        private: Private::Keyword("local"),
        file_refs: true,
        dynamic: LUA_DYNAMIC,
        ..BASE
    },
    Spec {
        name: "Elixir",
        exts: &["ex", "exs"],
        line_comments: &["#"],
        block_comment: None,
        multiline: true,
        triple: true,
        block: Block::End,
        defs: &[
            (r"^\s*defp\s+(?P<name>[a-z_]\w*)", Function),
            (r"^\s*def\s+(?P<name>[a-z_]\w*)", Method),
            (r"^\s*defmodule\s+(?:[\w.]+\.)?(?P<name>[A-Z]\w*)", Type),
        ],
        keep: &["start", "init", "child_spec", "handle_call", "handle_cast", "handle_info", "terminate", "main"],
        private: Private::Keyword("defp"),
        ..BASE
    },
    Spec {
        name: "Shell",
        exts: &["sh", "bash", "zsh"],
        line_comments: &["#"],
        block_comment: None,
        multiline: true,
        defs: &[
            (r"^(?:function\s+)?(?P<name>[A-Za-z_]\w*)\s*\(\)", Function),
            (r"^function\s+(?P<name>[A-Za-z_]\w*)\s*\{", Function),
        ],
        keep: &["main", "usage", "cleanup"],
        syntax: &["bash", "-n"],
        ..BASE
    },
];

static LANGS: OnceLock<Vec<Lang>> = OnceLock::new();

pub fn all() -> &'static [Lang] {
    LANGS.get_or_init(|| {
        SPECS
            .iter()
            .map(|spec| Lang {
                spec,
                defs: spec.defs.iter().map(|(re, kind)| (Regex::new(re).expect("definition pattern"), *kind)).collect(),
            })
            .collect()
    })
}

/// The language of a file, from its extension or name.
pub fn for_path(rel: &str) -> Option<&'static Lang> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    all().iter().find(|l| l.spec.names.contains(&name) || (!ext.is_empty() && l.spec.exts.contains(&ext.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles_and_names_its_group() {
        for lang in all() {
            assert!(!lang.spec.exts.is_empty(), "{} has no extensions", lang.spec.name);
            for (re, _) in &lang.defs {
                assert!(
                    re.capture_names().flatten().any(|n| n == "name"),
                    "{}: {re} has no name group",
                    lang.spec.name
                );
            }
        }
    }

    #[test]
    fn extensions_are_unique() {
        let mut seen = std::collections::HashMap::new();
        for spec in SPECS {
            for ext in spec.exts {
                assert!(seen.insert(ext, spec.name).is_none(), ".{ext} is claimed twice");
            }
        }
    }

    #[test]
    fn finds_languages() {
        assert_eq!(for_path("src/a/b.PY").unwrap().spec.name, "Python");
        assert_eq!(for_path("Rakefile").unwrap().spec.name, "Ruby");
        assert_eq!(for_path("x.luau").unwrap().spec.name, "Luau");
        assert!(for_path("README.md").is_none());
    }
}
