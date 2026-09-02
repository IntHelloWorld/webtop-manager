## Summary

Describe the user-visible outcome and the boundary affected by this change.

## Verification

- [ ] `./scripts/check.sh`
- [ ] `cargo check --package webtop-manager`
- [ ] Controller asset rebuilt and `zstd --test --quiet` passed, or not applicable
- [ ] No secrets, private host paths, generated packages, or application data added
- [ ] Security, architecture, and status docs updated, or not applicable

## Security review

Explain changes to Docker access, filesystem access, network exposure, secret
handling, destructive operations, or state recovery. Write “none” if this pull
request does not affect those areas.
