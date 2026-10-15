#!/usr/bin/env python3

import unittest

from prune_actions_caches import CacheEntry, family, plan_deletions


LOCK = "61003fbad5da9f11ea32c112908e91ba95c7f9077803c0eafe0f170cb88920ff"
TOOLCHAIN = "570656042681cfd8795403a455baf9a33035331a07db0645e866bbcea89a3d64"
SHA = "ed5d83a71fc4307b0ddcfe2a052b4848c01a890f"


def entry(id: int, key: str, created_at: str, ref: str = "refs/heads/main") -> CacheEntry:
    return CacheEntry(id=id, key=key, ref=ref, size_in_bytes=1, created_at=created_at)


class FamilyTest(unittest.TestCase):
    def test_run_scoped_keys(self) -> None:
        cases = {
            f"sccache-windows-2022-x86_64-pc-windows-msvc-dev-{LOCK}-38660201097": (
                "sccache-windows-2022-x86_64-pc-windows-msvc-dev"
            ),
            "sccache-tmux-smoke-Linux-ad92baf5dbb36bcc-38646905550": "sccache-tmux-smoke-Linux",
            "setup-zig-cache-v2-clippy-zig-linux-x86_64-0.14.0--38660949984-1": (
                "setup-zig-cache-v2-clippy-zig-linux-x86_64-0.14.0-"
            ),
            f"codeql-overlay-base-database-1-8a3da6f5ad0a4cf6-cpp-2.27.2-{SHA}-38661942532-1": (
                "codeql-overlay-base-database-1-8a3da6f5ad0a4cf6-cpp-2.27.2"
            ),
        }
        for key, expected in cases.items():
            with self.subTest(key=key):
                self.assertEqual(family(key), expected)

    def test_keys_saved_only_on_miss_are_not_run_scoped(self) -> None:
        for key in (
            f"cargo-home-windows-2022-x86_64-pc-windows-msvc-dev-{LOCK}-{TOOLCHAIN}",
            "apt-ubuntu-24.04-x86_64-unknown-linux-musl-v1",
            "setup-zig-tarball-zig-linux-x86_64-0.14.0",
            "v0-rust-real-tui-Linux-x64-14d2a2b0-958071ca",
            f"argument-comment-lint-Linux-5.0.0-5.0.0-{LOCK}",
        ):
            with self.subTest(key=key):
                self.assertIsNone(family(key))


class PlanDeletionsTest(unittest.TestCase):
    def test_keeps_newest_copy_per_family(self) -> None:
        old = entry(1, f"sccache-core-security-macos-15-{LOCK}-38660201097", "2026-10-15T00:38:45Z")
        other_lock = entry(2, f"sccache-core-security-macos-15-{TOOLCHAIN}-38660201154", "2026-10-15T00:39:44Z")
        newest = entry(3, f"sccache-core-security-macos-15-{LOCK}-38660949984", "2026-10-15T00:47:28Z")
        other = entry(4, "sccache-tmux-smoke-Linux-ad92baf5dbb36bcc-38646905550", "2026-10-14T22:48:37Z")
        self.assertEqual(plan_deletions([newest, old, other, other_lock]), [other_lock, old])

    def test_ignores_other_refs_and_unscoped_keys(self) -> None:
        entries = [
            entry(1, "sccache-tmux-smoke-Linux-ad92baf5dbb36bcc-38646905550", "2026-10-14T00:00:00Z", ref="refs/pull/1/merge"),
            entry(2, "sccache-tmux-smoke-Linux-ad92baf5dbb36bcc-38646905551", "2026-10-15T00:00:00Z"),
            entry(3, f"cargo-home-macos-dev-{LOCK}-{TOOLCHAIN}", "2026-10-14T00:00:00Z"),
            entry(4, f"cargo-home-macos-dev-{TOOLCHAIN}-{LOCK}", "2026-10-15T00:00:00Z"),
        ]
        self.assertEqual(plan_deletions(entries), [])


if __name__ == "__main__":
    unittest.main()
