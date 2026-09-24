# Permissions and trust

A module is code somebody else wrote. Before Limen runs one, it tells you exactly
what that module asked for, and nothing runs until you agree.

## What a module can ask for

- **Spawn local processes** — it runs other programs on this machine.
- **Network** — and which hosts. A module limited to `api.crowdstrike.com` cannot
  reach anywhere else.
- **Filesystem** — which paths. `<user-selected>` means only files you pick
  yourself in a dialog.
- **Run as administrator** — it needs elevation on this machine to do its job.
- **Execute on fleet hosts** — it runs code on *remote* machines, through an
  endpoint agent. This is the heaviest thing on the list.

## Trust is pinned to content

When you trust a module, Limen records the digest of the exact files it trusted.
Change those files — an update, an edit, or something you did not do — and the
trust no longer applies. You are asked again, because it is no longer the same
module.

`limen-cli verify` checks every installed module against those recorded digests
in one go. On a portable install carried between machines, that is the answer to
"is this still what I put on the stick".

## Read-only is not a permission

A module whose script only reads still declares that it runs code, because
"read-only" is a property of the script, not of the permission. Judge the
permission, not the promise.
