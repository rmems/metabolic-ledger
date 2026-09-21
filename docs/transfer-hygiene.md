# Repository transfer hygiene

The canonical repository is
[`rmems/metabolic-ledger`](https://github.com/rmems/metabolic-ledger). Package
metadata, installation examples, and the GitHub Container Registry workflow use
that owner and repository name.

## Transfer audit (2026-09-21)

- **Issues and pull requests:** GitHub issue
  [#22](https://github.com/rmems/metabolic-ledger/issues/22) tracks this audit.
  No overlapping pull request was open when this work began. Existing issue and
  pull-request history remains in the transferred repository.
- **GitHub Actions:** The CI and Docker workflows are present after the transfer.
  Pull requests continue to build the Docker image without logging in or pushing;
  pushes to `main` retain the existing GHCR publication behavior and permissions.
  The image target is now `ghcr.io/rmems/metabolic-ledger`. Registry publication
  must be verified separately after this change merges; this audit does not
  publish an image or change repository tokens or settings.
- **Wiki:** The repository wiki is enabled and populated under
  [`rmems/metabolic-ledger.wiki.git`](https://github.com/rmems/metabolic-ledger/wiki).
  Its documentation still mentions the former GHCR namespace. Updating those
  separate wiki pages is an explicit follow-up and is not represented as complete
  by this repository change.

Copyright notices and the Azure DevOps URL retain `Limen-Neural` intentionally:
they record authorship and the location of an external historical pipeline rather
than identifying this repository's current GitHub owner.
