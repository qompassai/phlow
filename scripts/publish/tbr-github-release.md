# HUMAN-GATED: tag the release and cut the GitHub release

Matt runs these steps himself after `tbr-crates-publish.sh` succeeds.
`VERSION` below is the version just published (e.g. `0.3.0`).

## 1. Tag the release commit

The release commit is the commit that bumped every crate to `VERSION`
plus the CHANGELOG entry. Tag it and push the tag:

```sh
git tag -a "vVERSION" -m "phlow VERSION"
git push origin "vVERSION"
```

Verify the tag landed:

```sh
git ls-remote origin "refs/tags/vVERSION"
```

## 2. Attach the Linux tarball to a GitHub release

```sh
scripts/publish/package-linux.sh   # builds dist/phlow-VERSION-x86_64-unknown-linux-gnu.tar.gz
gh release create "vVERSION" \
  --title "phlow VERSION" \
  --notes-file <(sed -n '/## \[VERSION\]/,/## \[/p' CHANGELOG.md | head -n -1) \
  dist/phlow-VERSION-x86_64-unknown-linux-gnu.tar.gz \
  dist/phlow-VERSION-x86_64-unknown-linux-gnu.tar.gz.sha256
```

(Replace `VERSION` with the real version in each command.)

## 3. Verify docs.rs

For each published crate, confirm it built on docs.rs:

- `https://docs.rs/phlow-cli/VERSION`
- repeat for the other crates, or spot-check the leaves first

Re-check any crate that shows a docs.rs build failure.

## 4. Announce

Post the release notes wherever Matt announces releases. The CHANGELOG
entry under `## [VERSION]` is the source of truth.
