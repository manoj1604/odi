# Signing and trust

An ODI file's digest already proves its contents: every reader checks every
byte it uses against it. A signature adds who vouches for that digest.

## Sign

```sh
odi key new               # ~/.odi/keys/default.key; prints the public key
odi sign app.odi          # adds a signature; the digest doesn't change
odi inspect app.odi       # lists signatures
```

`odi key new NAME` makes more keys and `odi sign --key NAME` uses one;
`odi key ls` lists them with their public keys. A public key looks like
`ed25519:3f1c…` (64 hex digits). Give it to whoever runs your files.

A file can carry several signatures. Signing doesn't change the digest, so
pushing a signed file again only sends its last piece.

## Check

```sh
odi verify app.odi --key ed25519:3f1c…
```

`verify` checks every byte against the digest and every signature against
it; with `--key`, one of those keys must have signed it.

## Trust

A runner that trusts keys runs only files one of them signed:

```sh
odi run acme/web:1.4 --trust ed25519:3f1c…
```

Trusted keys come from, together:

- `--trust KEY` (repeatable) on `run`, `exec` and `mcp`;
- `ODI_TRUST`, keys separated by commas;
- `~/.odi/trust`, one key per line (`#` starts a comment).

With no trusted keys, everything runs. A set's signature covers its parts,
because the set pins each part by digest. Parts added with `odi attach` are
checked too.

## Format

Signatures are ed25519 over `odi-signature-v0\n` followed by the 32-byte
digest, kept in a block just before the file's footer: see the
[specification, §3.5](../SPEC.md#35-signatures-optional).
