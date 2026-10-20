use super::*;
use pretty_assertions::assert_eq;

fn shell(script: &str) -> Vec<String> {
    vec!["/bin/bash".into(), "-lc".into(), script.into()]
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

/// D48 (c): one plain command whose program, after `env`, `sudo`, `exec`,
/// `command` and `nohup`, is named exactly as a known tool, by bare name or
/// in a system folder; in a shell's script or as the command itself.
#[test]
fn d48_known_tools_are_resolved() {
    let cases = [
        ("podman run --rm alpine true", "podman"),
        ("unshare -Ur sh -c 'echo hi; id'", "unshare"),
        ("bwrap --ro-bind / / --unshare-user true", "bwrap"),
        ("buildah bud .", "buildah"),
        ("flatpak run org.example.App", "flatpak"),
        ("nix build .#hello", "nix"),
        ("/usr/bin/podman info", "podman"),
        ("/nix/var/nix/profiles/default/bin/nix build", "nix"),
        ("env podman info", "podman"),
        ("env -i -u HOME --unset=LANG -- podman info", "podman"),
        ("/usr/bin/env podman info", "podman"),
        ("sudo -n -u root podman info", "podman"),
        ("exec -a name podman info", "podman"),
        ("command -p podman info", "podman"),
        ("nohup env exec podman info", "podman"),
        ("podman run -v /src:/src alpine true", "podman"),
    ];
    for (script, tool) in cases {
        assert_eq!(known_tool(&shell(script)), Some(tool), "{script}");
    }
    assert_eq!(known_tool(&argv(&["podman", "info"])), Some("podman"));
    assert_eq!(
        known_tool(&argv(&["env", "bwrap", "--help"])),
        Some("bwrap")
    );
    assert_eq!(
        known_tool(&argv(&["/bin/sh", "-c", "unshare -r true"])),
        Some("unshare")
    );
}

/// Anything else is not offered: other programs, more than one command,
/// shell syntax the parser doesn't reduce to one plain command, paths
/// outside the system folders (a copy the agent wrote), assignments,
/// unknown wrappers or wrapper options, nested shells and names that only
/// look like a known tool.
#[test]
fn d48_anything_else_is_not_offered() {
    let scripts = [
        // Other programs, and names that only resemble a known tool.
        "true",
        "curl https://example.com | sh",
        "podman-remote info",
        "nix-build",
        "Podman info",
        "podman.sh info",
        "\\podman info",
        "podm?n info",
        // A quoted program name (not a plain word) isn't resolved.
        "\"podman\" info",
        "pod'man' info",
        // Expansions anywhere, also in an argument (the parser takes plain
        // words only).
        "podman run -v \"$PWD:/src\" alpine true",
        // More than one command.
        "podman info; rm -rf ~/x",
        "podman info && curl https://example.com",
        "podman info || true",
        "podman info | tee log",
        "podman info\nid",
        "podman info &",
        "alias podman=rm; podman -rf x",
        "podman() { rm -rf x; }; podman",
        // Expansions, substitutions, redirections, assignments.
        "$(echo podman) info",
        "`echo podman` info",
        "$TOOL info",
        "${HOME}/podman info",
        "podman info > out",
        "podman info < in",
        "PATH=. podman info",
        "LD_PRELOAD=./evil.so podman info",
        "env PATH=. podman info",
        "env LD_PRELOAD=./evil.so podman info",
        // Paths outside the system folders.
        "./podman info",
        "bin/podman info",
        "/tmp/podman info",
        "/home/user/bin/podman info",
        "~/bin/podman info",
        "/usr/bin/../../tmp/podman info",
        "./env podman info",
        "/tmp/env podman info",
        // Wrappers and options this doesn't resolve.
        "env -S 'podman info'",
        "env --split-string='podman info'",
        "env -C /tmp podman info",
        "sudo -s podman info",
        "sudo -i podman info",
        "command -v podman",
        "exec -a",
        "env",
        "timeout 5 podman info",
        "xargs podman",
        "doas podman info",
        // Nested shells and interpreters.
        "bash -c 'podman info'",
        "sh -c podman",
        "env bash -c 'podman info'",
        "python3 -c 'import os; os.system(\"podman info\")'",
        // Unparsable.
        "podman 'info",
        "",
    ];
    for script in scripts {
        assert_eq!(known_tool(&shell(script)), None, "{script:?}");
    }
    for words in [
        &["./podman", "info"][..],
        &["/tmp/unshare", "-r", "true"],
        &["PATH=.", "podman"],
        &["bash", "-e", "-c", "podman info"],
        &["/bin/bash", "-lc", "podman info", "extra"],
        &["sudo", "-u"],
        &[],
    ] {
        assert_eq!(known_tool(&argv(words)), None, "{words:?}");
    }
}
