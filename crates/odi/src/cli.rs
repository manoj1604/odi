//! What `odi help`, `odi help COMMAND` and `odi COMMAND --help` say: one
//! table of commands (docs/cli.md is made from it: docs/gen-cli.sh), and
//! the flags each command defines where it parses them.

use crate::flags::{Kind, Spec};

pub const DOCS: &str = "https://github.com/manoj1604/odi/tree/main/docs";

pub struct Command {
    pub name: &'static str,
    pub group: &'static str,
    /// Usage lines, without "odi ".
    pub usage: &'static [&'static str],
    /// One line, for the command list.
    pub about: &'static str,
    /// More, for its own help (may be empty).
    pub details: &'static str,
    pub examples: &'static [&'static str],
    /// Whether it parses flags (then `--help` is answered where it does,
    /// with its flags); otherwise main answers it.
    pub flags: bool,
}

const fn c(
    name: &'static str,
    group: &'static str,
    usage: &'static [&'static str],
    about: &'static str,
    details: &'static str,
    examples: &'static [&'static str],
    flags: bool,
) -> Command {
    Command { name, group, usage, about, details, examples, flags }
}

pub const GROUPS: &[&str] =
    &["Build", "Run", "Parts and sets", "Fast starts", "Inspect and trust", "Registry", "This command"];

/// What `odi help` shows: the few commands most people need. The others
/// work as they always did, listed by `odi help --all` as experimental
/// (TODO.md: what they're waiting for).
pub const CORE: &[(&str, &[&str])] = &[
    ("Build and run", &["init", "build", "run", "bundle"]),
    ("Look inside", &["inspect", "hook", "close"]),
    ("Publish on the hub", &["login", "push", "pull", "versions", "logout"]),
    ("Help", &["help", "version"]),
];

pub fn is_core(name: &str) -> bool {
    CORE.iter().any(|(_, names)| names.contains(&name))
}

pub const COMMANDS: &[Command] = &[
    c(
        "init",
        "Build",
        &["init [PROJECT] [flags]"],
        "Write the project's odi.json, from what odi finds in it",
        "Every project odi builds has an odi.json: its name, its type (go, node, nextjs, python, static) and anything odi can't work out (the runtime, whether to include it, build and start commands, the port, environment variables). odi init writes one with what it finds; edit it as you like. Specification: docs/odi-json.md.",
        &["odi init", "odi init --type static", "odi init ../shop --name shop"],
        true,
    ),
    c(
        "build",
        "Build",
        &["build [PROJECT] [flags]"],
        "Build a project into an ODI file (Go, Node.js, Next.js, Python, static)",
        "Runs the project's own build (go build, npm ci and npm run build, pip) and packs \
         what it produced, with a manifest that says how to start it. The same commit builds to \
         the same digest.\n\n\
         It needs the project's odi.json (odi init writes one): its name and type, and \
         anything odi can't work out:\n  \
         {\"name\": \"shop\", \"type\": \"nextjs\", \"runtime\": \"node 22\", \"include_runtime\": true,\n   \
         \"env\": {\"DATABASE_URL\": \"required\"}}\n\
         include_runtime puts Node.js itself in the file (the official release for the target, \
         checked), so it runs where Node.js isn't installed. Every key: docs/odi-json.md.\n\n\
         --platform builds for another machine: Go, Node.js and Next.js swap compiled packages \
         for the target's, at the lockfile's versions; Python builds for this machine.",
        &[
            "odi build",
            "odi build ./api -o api.odi",
            "odi build --platform linux/amd64",
            "odi build --runtime node-22.23.3-linux-arm64.odi",
            "odi build --profile --path / --path /pricing",
        ],
        true,
    ),
    c(
        "pack",
        "Build",
        &["pack DIR [flags]"],
        "Pack a folder already laid out (app/, static/, runtime/, meta/)",
        "For build output odi doesn't build itself. The manifest comes from \
         DIR/odi.manifest.json (or --manifest) and isn't packed as a file.",
        &["odi pack out/ -o site.odi", "odi pack dist --manifest odi.manifest.json --compress max"],
        true,
    ),
    c(
        "run",
        "Run",
        &["run FILE|NAME [flags]"],
        "Run an ODI file, or one from a registry, on this machine",
        "Static files are served by odi; requests for anything else go to the web process. \
         A set's parts are put together first, and parts that provide http or tcp plugs run as \
         services beside it.\n\n\
         From a registry, on Linux, the file is mounted (FUSE, fusermount3) and read as the \
         program asks for it, starting with its start list; elsewhere it's fetched whole first. \
         --trust (or ~/.odi/trust, $ODI_TRUST) refuses anything not signed by those keys.",
        &[
            "odi run app.odi",
            "odi run acme/web:1.4 --port 3000 -e DATABASE_URL=postgres://...",
            "odi run acme/web:1.4 --trust ed25519:3f1c...",
        ],
        true,
    ),
    c(
        "exec",
        "Run",
        &["exec FILE|NAME [flags] [-- ARGS...]"],
        "Run one of its processes in this terminal (a command-line tool)",
        "Arguments after -- go to the program. Its exit code is odi's.",
        &["odi exec acme/pdf-tools:2 -- merge a.pdf b.pdf -o out.pdf", "odi exec set.odi --part lint -- --fix ."],
        true,
    ),
    c(
        "mcp",
        "Run",
        &["mcp FILE|NAME [flags]"],
        "Serve what it provides as an mcp plug, as a stdio MCP server (for AI agents)",
        "The command an agent's MCP configuration names: \
         {\"command\": \"odi\", \"args\": [\"mcp\", \"acme/pdf:1\"]}.",
        &["odi mcp acme/pdf:1", "odi mcp tools.set.odi --name search"],
        true,
    ),
    c(
        "attach",
        "Run",
        &["attach RUNNING PART [flags]"],
        "Add a part to a set that's running",
        "RUNNING is the running set's name (its manifest name, as odi inspect shows it). Linux and \
         macOS. The part's services start, and its plugs are \
         added to the file $ODI_PLUGS names, which every process of the set can read.",
        &["odi attach web prices.odi --as prices", "odi attach web acme/search:3"],
        true,
    ),
    c(
        "web",
        "Run",
        &["web FILE... [flags]"],
        "Run a browser app (its static/ files) in a browser tab",
        "Serves the browser runner (built into odi) and the files on localhost. The tab reads \
         the ODI file with Range requests, fetching only what the app asks for, and checks \
         every byte; it can keep the app for offline play. Any static host with Range requests \
         can serve the runner (web/) and ODI files the same way.",
        &["odi web game.odi", "odi web game.odi mod.set.odi --port 9000"],
        true,
    ),
    c(
        "set",
        "Parts and sets",
        &["set MAIN [flags]"],
        "Make a set: parts pinned by digest, put together when it runs",
        "A set is a small ODI file whose manifest lists other ODI files (parts) by digest. \
         main, runtime and overlay parts are merged into one tree, a later file replacing an \
         earlier one; a --part NAME=PART is under parts/NAME/. PART is an ODI file or a name in \
         a registry.",
        &[
            "odi set web.odi --runtime acme/node:22",
            "odi set game.odi --overlay hd-textures.odi -o game-hd.set.odi",
            "odi set web.odi --part prices=prices.odi --part docs=acme/docs:1",
        ],
        true,
    ),
    c(
        "runtime",
        "Parts and sets",
        &["runtime node [VERSION] [flags]", "runtime NAME VERSION DIR [flags]"],
        "Make a runtime part: one copy serves every app that pins it",
        "odi runtime node downloads nodejs.org's release for the platform, checks it against \
         the published SHA-256, and packs it: the same release gives the same digest on any \
         machine. NAME VERSION DIR packs any folder with bin/.",
        &["odi runtime node 22.23.3 --platform linux/arm64", "odi runtime python 3.13.1 ./python-3.13.1"],
        true,
    ),
    c(
        "part",
        "Parts and sets",
        &["part add FILE...", "part ls"],
        "This machine's store of parts",
        "Sets find their parts here (by digest) before asking a registry.",
        &["odi part add node-22.23.3-linux-arm64.odi", "odi part ls"],
        false,
    ),
    c(
        "profile",
        "Fast starts",
        &["profile FILE [flags]", "profile FILE --mcp [--input FILE] [-o FILE]"],
        "Record what it reads while starting: its start list (Linux)",
        "Runs it mounted, asks for each --path once it's up (or, with --mcp, sends a stdio \
         tool initialize and tools/list), and keeps what it read, in order, in the file. Run \
         from a registry, all of that then comes in one request.",
        &["odi profile app.odi --path / --path /docs", "odi profile tool.odi --mcp"],
        true,
    ),
    c(
        "fill",
        "Fast starts",
        &["fill NAME|FILE... [flags]"],
        "Fetch its start list (or all of it) onto this machine before its first run",
        "For hosts that know what they'll run: the first request then reads nothing from \
         the network.",
        &["odi fill acme/web:1.4", "odi fill --all acme/web:1.4 acme/api:2"],
        true,
    ),
    c(
        "inspect",
        "Inspect and trust",
        &["inspect FILE|NAME"],
        "Show what's in it: processes, static files, runtime, parts, signatures, size",
        "",
        &["odi inspect app.odi", "odi inspect acme/web:1.4"],
        false,
    ),
    c(
        "verify",
        "Inspect and trust",
        &["verify FILE [flags]"],
        "Check every byte against its digest, and its signatures",
        "With --key, one of those keys must have signed it. A set's parts are checked too.",
        &["odi verify app.odi", "odi verify app.odi --key ed25519:3f1c..."],
        true,
    ),
    c(
        "diff",
        "Inspect and trust",
        &["diff OLD NEW [flags]"],
        "Compare two builds: what changed, and what a machine with the old one must fetch",
        "OLD and NEW are ODI files or names in a registry (only their indexes are read).",
        &["odi diff acme/web:1.4.2 acme/web:1.4.3", "odi diff old.odi new.odi --all"],
        true,
    ),
    c(
        "extract",
        "Inspect and trust",
        &["extract FILE DIR"],
        "Unpack it into a folder (a set: put together first)",
        "",
        &["odi extract app.odi out/"],
        false,
    ),
    c(
        "bundle",
        "Run",
        &["bundle FILE [-o OUT] [--platform OS/ARCH] [--runner FILE]"],
        "Make one executable that runs the app, with nothing to install where it runs",
        "Writes a small static odi for the target machine with the ODI file after it, unchanged (the same digest). For a machine with no Node.js, build the app with its runtime inside (\"include_runtime\": true in odi.json). Copy it to a VM, a microVM or a server and run it: it serves the app as odi run would (--port, -e KEY=VALUE). OUT --odi-file app.odi gets the ODI file back out. The runner is this odi when it fits the target, else odi's release for it, checked against its SHA256SUMS.",
        &["odi bundle app.odi -o app && scp app vm: && ssh vm ./app", "odi bundle app.odi --platform linux/amd64"],
        true,
    ),
    c(
        "close",
        "Run",
        &["close [FILE|NAME]"],
        "Let go of the files cd went into",
        "",
        &["odi close", "odi close app.odi"],
        false,
    ),
    c(
        "shell",
        "Run",
        &["shell FILE|NAME [PATH] [flags] [-- COMMAND...]"],
        "Open a shell inside it, ready to start it by hand (cd into the file)",
        "You land in the app's folder (app/) with everything odi run would give it: the file's runtime first on PATH (node, python), its environment, PORT, and its writable folders. odi prints the start command; when the app has static files (Next.js's /_next/static), odi serves them on --port (8080) and passes the rest to the app, as odi run does. On Linux the file is mounted read-only and read as you go (FUSE); elsewhere you get a copy of your own, removed when you exit. ODI_ROOT is the file's top.",
        &["odi shell app.odi            # then: node server.js, and open http://localhost:8080", "odi shell acme/web:1.4 -e DATABASE_URL=postgres://localhost/web", "odi shell app.odi static     # look around the static files", "odi shell app.odi -- node -e 'console.log(process.version)'"],
        true,
    ),
    c(
        "key",
        "Inspect and trust",
        &["key new [NAME]", "key ls"],
        "Make or list signing keys (ed25519, in ~/.odi/keys)",
        "Prints the public key, ed25519:HEX, to give to whoever runs your files.",
        &["odi key new", "odi key new release", "odi key ls"],
        false,
    ),
    c(
        "sign",
        "Inspect and trust",
        &["sign FILE... [flags]"],
        "Sign it with one of your keys (its digest doesn't change)",
        "",
        &["odi sign app.odi", "odi sign app.odi --key release"],
        true,
    ),
    c(
        "login",
        "Registry",
        &["login [flags]"],
        "Sign in to a registry",
        "Without --token, asks for one. The default registry is hub.odisssi.com; \
         --registry (or $ODI_REGISTRY) names another.",
        &["odi login", "odi login --registry https://registry.example.com --token $TOKEN"],
        true,
    ),
    c("logout", "Registry", &["logout"], "Forget the registry sign-in", "", &["odi logout"], false),
    c(
        "push",
        "Registry",
        &["push [NAMESPACE/NAME[:VERSION]] [FILE...] [flags]"],
        "Publish a version (one build per platform), and move latest to it",
        "Only what the registry doesn't have yet is sent. The version is the one named \
         (acme/web:1.4.2), or the next after the newest (--bump patch|minor|major), or the \
         file's own (odi build takes it from a git tag, package.json or pyproject.toml). A \
         release also moves latest; a pre-release (2.0.0-rc.1) doesn't.\n\n\
         A published version doesn't change: pushing other builds to it is refused (--force \
         moves it). Other tags (latest, staging) move. With several files, each is the build \
         for its platform; --add adds platforms to a version one CI job at a time.\n\n\
         Without a name, the repository is odi.json's \"repo\" (\"repo\": \"acme/web\").",
        &[
            "odi push acme/web",
            "odi push acme/web:1.4.2",
            "odi push acme/web --bump minor",
            "odi push acme/web:2.0.0-rc.1 --tag next",
            "odi push acme/web:1.4.2 web-amd64.odi web-arm64.odi",
            "odi push acme/web:1.4.2 web-arm64.odi --add",
        ],
        true,
    ),
    c(
        "pull",
        "Registry",
        &["pull NAMESPACE/NAME[:TAG|:RANGE|@sha256:DIGEST] [flags]"],
        "Download a file (this machine's platform), checked against its digest",
        "Any command that takes a name takes a version range too: acme/web:1.4 is the newest \
         1.4.x, acme/web:1 (or ^1.0) the newest 1.x.y, ~1.4.2 the newest 1.4.x from 1.4.2. \
         A tag with that exact name wins.",
        &["odi pull acme/web", "odi pull acme/web:1.4", "odi pull acme/web@sha256:877a... -o web.odi"],
        true,
    ),
    c(
        "versions",
        "Registry",
        &["versions [NAMESPACE/NAME] [flags]"],
        "List a repository's versions and tags: platforms, size, when",
        "Newest version first, with the other tags pointing at the same builds (latest, \
         staging). Without a name, odi.json's \"repo\". (odi tags is the same.)",
        &["odi versions acme/web", "odi versions acme/web --all"],
        true,
    ),
    c(
        "tag",
        "Registry",
        &["tag NAMESPACE/NAME:FROM TAG... [flags]"],
        "Point tags at a version's builds, without pushing again (promote, roll back)",
        "FROM is a version, a range, a tag or @sha256:DIGEST. Versions already published \
         don't move without --force.",
        &["odi tag acme/web:1.4.2 latest", "odi tag acme/web:1.3 latest", "odi tag acme/web:staging production"],
        true,
    ),
    c(
        "search",
        "Registry",
        &["search [QUERY]"],
        "Find public repositories (by name or language)",
        "",
        &["odi search", "odi search nextjs"],
        false,
    ),
    c("repos", "Registry", &["repos"], "List your organization's repositories", "", &["odi repos"], false),
    c(
        "rm",
        "Registry",
        &["rm NAMESPACE/NAME:TAG", "rm NAMESPACE/NAME@sha256:DIGEST", "rm NAMESPACE/NAME --repo"],
        "Delete a tag, a build, or a repository with its builds",
        "",
        &["odi rm acme/web:old", "odi rm acme/scratch --repo"],
        true,
    ),
    c(
        "usage",
        "Registry",
        &["usage [NAMESPACE]"],
        "Show what your organization stores, and its limits",
        "",
        &["odi usage"],
        false,
    ),
    c(
        "help",
        "This command",
        &["help [COMMAND]"],
        "Show help for odi, or for one command",
        "",
        &["odi help run"],
        false,
    ),
    c(
        "hook",
        "This command",
        &["hook zsh|bash|fish"],
        "Set up cd into .odi files: cd app.odi (add its line to your shell)",
        "Add it to your shell's startup file:\n  \
         zsh:  eval \"$(odi hook zsh)\"    (in ~/.zshrc)\n  \
         bash: eval \"$(odi hook bash)\"   (in ~/.bashrc)\n  \
         fish: odi hook fish | source       (in ~/.config/fish/config.fish)\n\
         Then cd app.odi, cd app.odi/app or cd acme/web:1.4 (a hub name with a tag) goes inside, read-only, and ls, cat and the rest work as usual. On Linux the file is mounted and read as you look (FUSE); elsewhere it's unpacked once under ~/.odi/open. odi close lets them go. A cd to a folder works as it always did.",
        &["odi hook zsh"],
        false,
    ),
    c(
        "completions",
        "This command",
        &["completions bash|zsh|fish"],
        "Print a shell completion script",
        "Add it to your shell's startup file, for example:\n  \
         bash: eval \"$(odi completions bash)\"   (in ~/.bashrc)\n  \
         zsh:  eval \"$(odi completions zsh)\"    (in ~/.zshrc, after compinit)\n  \
         fish: odi completions fish > ~/.config/fish/completions/odi.fish",
        &["odi completions zsh"],
        false,
    ),
    c("version", "This command", &["version"], "Show odi's version", "", &["odi version"], false),
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// `odi help`: the commands, by group.
pub fn overview(all: bool) -> String {
    let mut s = format!(
        "odi {}: one file for a program's build output, any language, runs anywhere\n\n\
         Usage: odi COMMAND [ARGUMENTS] [FLAGS]\n",
        crate::VERSION
    );
    for (g, names) in CORE {
        s.push_str(&format!("\n{g}:\n"));
        for c in names.iter().filter_map(|n| find(n)) {
            s.push_str(&format!("  {:<13}{}\n", c.name, c.about));
        }
    }
    if all {
        s.push_str("\nExperimental (they work, and may change before they're in this list; see TODO.md):\n");
        for g in GROUPS {
            let more: Vec<&Command> = COMMANDS.iter().filter(|c| c.group == *g && !is_core(c.name)).collect();
            if more.is_empty() {
                continue;
            }
            s.push_str(&format!("\n{g}:\n"));
            for c in more {
                s.push_str(&format!("  {:<13}{}\n", c.name, c.about));
            }
        }
    }
    s.push_str("\nRun 'odi help COMMAND' (or odi COMMAND --help) for its flags and examples.\n");
    if !all {
        s.push_str("More commands, experimental: odi help --all\n");
    }
    s.push_str(&format!("Docs: {DOCS}\n"));
    s
}

/// A flag's value, as help shows it: the usage line's word for it when
/// that's a placeholder (FILE, OS/ARCH), else a name for what it takes.
fn metavar(c: &Command, sp: &Spec) -> String {
    let dashes = if sp.name.len() == 1 { "-" } else { "--" };
    let lead = format!("{dashes}{} ", sp.name);
    for u in c.usage {
        if let Some(i) = u.find(&lead) {
            let word: String =
                u[i + lead.len()..].chars().take_while(|ch| !matches!(ch, ' ' | ']' | '|' | '[')).collect();
            if !word.is_empty() && !word.chars().any(|ch| ch.is_ascii_lowercase()) {
                return word;
            }
        }
    }
    match sp.name {
        "o" | "input" | "manifest" => "FILE",
        "platform" => "OS/ARCH",
        "e" => "KEY=VALUE",
        "port" => "PORT",
        "path" => "PATH",
        "runtime" | "overlay" => "PART",
        "part" if c.name == "set" => "NAME=PART",
        "trust" => "KEY",
        "static" => "app|odi",
        "key" if c.name == "verify" => "KEY",
        "compress" => "LEVEL",
        "registry" => "URL",
        "token" => "TOKEN",
        "bump" => "PART",
        "tag" => "TAG",
        _ => "NAME",
    }
    .to_string()
}

/// A command's help: usage, what it does, its flags, examples.
pub fn command_help(c: &Command, specs: &[Spec]) -> String {
    let mut s = format!("odi {}: {}\n\nUsage:\n", c.name, c.about);
    for u in c.usage {
        s.push_str(&format!("  odi {u}\n"));
    }
    if !c.details.is_empty() {
        s.push('\n');
        s.push_str(&wrap(c.details, 78));
        s.push('\n');
    }
    if !specs.is_empty() {
        s.push_str("\nFlags:\n");
        let cols: Vec<(String, &Spec)> = specs
            .iter()
            .map(|sp| {
                let dashes = if sp.name.len() == 1 { "-" } else { "--" };
                let left = match sp.kind {
                    Kind::Bool => format!("{dashes}{}", sp.name),
                    _ => format!("{dashes}{} {}", sp.name, metavar(c, sp)),
                };
                (left, sp)
            })
            .collect();
        let w = cols.iter().map(|(l, _)| l.len()).max().unwrap_or(0).clamp(10, 26);
        for (left, sp) in cols {
            let rep = if sp.kind == Kind::Multi && !sp.help.contains("repeat") { " (repeatable)" } else { "" };
            if left.len() > w {
                s.push_str(&format!("  {left}\n  {:w$}  {}{rep}\n", "", sp.help));
            } else {
                s.push_str(&format!("  {left:w$}  {}{rep}\n", sp.help));
            }
        }
        s.push_str(&format!("  {:w$}  this help\n", "-h, --help"));
    }
    if !c.examples.is_empty() {
        s.push_str("\nExamples:\n");
        for e in c.examples {
            s.push_str(&format!("  {e}\n"));
        }
    }
    s
}

/// Text wrapped at width, keeping its own line breaks (and indented lines).
fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if para.starts_with(' ') {
            out.push_str(para);
            continue;
        }
        let mut line = 0;
        for word in para.split_whitespace() {
            if line > 0 && line + 1 + word.len() > width {
                out.push('\n');
                line = 0;
            } else if line > 0 {
                out.push(' ');
                line += 1;
            }
            out.push_str(word);
            line += word.len();
        }
    }
    out
}

/// The closest command name, for a mistyped one.
/// How many letters apart two words are (Levenshtein).
pub fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

pub fn suggest(name: &str) -> Option<&'static str> {
    COMMANDS.iter().map(|c| (distance(name, c.name), c.name)).filter(|(d, _)| *d <= 2).min().map(|(_, n)| n)
}

/// Shell completion: commands, and the words some take next.
pub fn completions(shell: &str) -> Option<String> {
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let words = names.join(" ");
    Some(match shell {
        "bash" => format!(
            "_odi() {{\n  local cur=${{COMP_WORDS[COMP_CWORD]}}\n  if [ \"$COMP_CWORD\" -eq 1 ]; then\n    \
             COMPREPLY=($(compgen -W \"{words}\" -- \"$cur\"))\n  else\n    case \"${{COMP_WORDS[1]}}\" in\n      \
             part) [ \"$COMP_CWORD\" -eq 2 ] && COMPREPLY=($(compgen -W \"add ls\" -- \"$cur\")) || COMPREPLY=($(compgen -f -- \"$cur\")) ;;\n      \
             key) COMPREPLY=($(compgen -W \"new ls\" -- \"$cur\")) ;;\n      \
             help) COMPREPLY=($(compgen -W \"{words}\" -- \"$cur\")) ;;\n      \
             completions) COMPREPLY=($(compgen -W \"bash zsh fish\" -- \"$cur\")) ;;\n      \
             *) COMPREPLY=($(compgen -f -- \"$cur\")) ;;\n    esac\n  fi\n}}\ncomplete -o filenames -F _odi odi\n"
        ),
        "zsh" => {
            let described: Vec<String> =
                COMMANDS.iter().map(|c| format!("    '{}:{}'", c.name, c.about.replace('\'', "'\\''"))).collect();
            format!(
                "#compdef odi\n_odi() {{\n  local -a cmds\n  cmds=(\n{}\n  )\n  if (( CURRENT == 2 )); then\n    \
                 _describe 'command' cmds\n  else\n    case $words[2] in\n      \
                 part) (( CURRENT == 3 )) && _values 'action' add ls || _files ;;\n      \
                 key) _values 'action' new ls ;;\n      \
                 help) _describe 'command' cmds ;;\n      \
                 completions) _values 'shell' bash zsh fish ;;\n      \
                 *) _files ;;\n    esac\n  fi\n}}\ncompdef _odi odi\n",
                described.join("\n")
            )
        }
        "fish" => {
            let mut s = String::from("complete -c odi -f\n");
            for c in COMMANDS {
                s.push_str(&format!(
                    "complete -c odi -n '__fish_use_subcommand' -a {} -d '{}'\n",
                    c.name,
                    c.about.replace('\'', "\\'")
                ));
            }
            s.push_str("complete -c odi -n '__fish_seen_subcommand_from part' -a 'add ls'\n");
            s.push_str("complete -c odi -n '__fish_seen_subcommand_from key' -a 'new ls'\n");
            s.push_str(&format!("complete -c odi -n '__fish_seen_subcommand_from help' -a '{words}'\n"));
            s.push_str("complete -c odi -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'\n");
            s.push_str(&format!(
                "complete -c odi -n 'not __fish_use_subcommand; and not __fish_seen_subcommand_from {}' -F\n",
                "part key help completions version logout"
            ));
            s
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "a command listed twice");
        for c in COMMANDS {
            assert!(GROUPS.contains(&c.group), "{}: group {}", c.name, c.group);
            assert!(c.usage.iter().all(|u| u.starts_with(c.name)), "{}: usage", c.name);
            assert!(!c.examples.is_empty() && c.examples.iter().all(|e| e.starts_with(&format!("odi {}", c.name))));
        }
        let all = overview(true);
        assert!(
            COMMANDS.iter().all(|c| all.contains(&format!("  {:<13}", c.name))),
            "odi help --all lists every command"
        );
        let core = overview(false);
        for (_, names) in CORE {
            assert!(names.iter().all(|n| find(n).is_some()), "a core command that doesn't exist");
        }
        assert!(!core.contains("  attach "), "odi help shows only the core");
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("bulid"), Some("build"));
        assert_eq!(suggest("pul"), Some("pull"));
        assert_eq!(suggest("zzzzzzzz"), None);
    }

    #[test]
    fn shells() {
        for sh in ["bash", "zsh", "fish"] {
            assert!(completions(sh).unwrap().contains("inspect"));
        }
        assert!(completions("csh").is_none());
    }
}
