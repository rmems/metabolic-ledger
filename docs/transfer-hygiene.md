# Repository transfer hygiene

The canonical repository is
[`rmems/metabolic-ledger`](https://github.com/rmems/metabolic-ledger). Package
metadata, installation examples, and the GitHub Container Registry workflow use
that owner and repository name.

## Transfer audit (2026-09-21)

Completed in PR
[#24](https://github.com/rmems/metabolic-ledger/pull/24):

- `Cargo.toml` `repository`, README git-dependency examples, and the Docker
  workflow image target were retargeted to `rmems/metabolic-ledger` /
  `ghcr.io/rmems/metabolic-ledger`.
- GitHub Actions CI and Docker workflows remained present after the transfer.
  Pull requests continue to build the Docker image without logging in or
  pushing; pushes to `main` retain GHCR publication behavior and permissions.
  Registry publication under the new name must still be verified separately on
  a `main` push; that check is outside this repository hygiene change.
- Issues and pull-request history remained in the transferred repository. No
  overlapping hygiene PR was open when #24 began.

## Completion audit (2026-11-09)

Re-checked against
[#22](https://github.com/rmems/metabolic-ledger/issues/22) acceptance. Repo-side
hygiene is complete; local shelf graduation remains a follow-up.

### Live docs and package metadata

| Location | Owner reference | Status |
|---|---|---|
| `Cargo.toml` `repository` | `https://github.com/rmems/metabolic-ledger` | current |
| `README.md` git dependency examples | `rmems/metabolic-ledger` | current |
| `.github/workflows/docker.yml` image | `ghcr.io/rmems/metabolic-ledger` | current |
| GitHub Actions workflows | `ci.yml`, `docker.yml` both `active` | current |
| Open issues / PRs | only #22 open for this hygiene work | coherent |

No remaining stale `Limen-Neural` **GitHub or GHCR** URLs were found in the
tree. Every live install, package, and registry pointer uses `rmems`.

### Intentional historical `Limen-Neural` mentions

These are **not** stale repository URLs. Keep them; do not rewrite to `rmems`:

| Location | Why retained |
|---|---|
| `LICENSE-MIT`, `LICENSE-APACHE-2.0` | Original copyright / authorship notice |
| `src/**/*.rs` SPDX copyright headers | Same authorship record |
| `azure-pipelines.yml` Azure DevOps URL | External historical pipeline host (`dev.azure.com/Limen-Neural`) |
| This document / `CLAUDE.md` Gotchas | Documents the keep-as-history policy |

### Wiki

- GitHub reports `has_wiki: true` for `rmems/metabolic-ledger`.
- The wiki remote resolves:
  [`rmems/metabolic-ledger.wiki.git`](https://github.com/rmems/metabolic-ledger/wiki)
  (HEAD reachable).
- Wiki Home and CI pages already name `rmems/metabolic-ledger` and
  `ghcr.io/rmems/metabolic-ledger`. Mentions of the former
  `ghcr.io/limen-neural/...` namespace are historical notes about the move, not
  live install targets.
- Further wiki-only edits stay optional and are not required to close repo
  hygiene.

### Out of scope (follow-up)

- Graduating the local clone from the limen-return shelf to flat `~/rmems/` is
  deliberately deferred by #22 and is not represented as complete here.
