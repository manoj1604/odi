# Security policy

odi runs programs and reads files that may come from strangers, so
security problems matter. Thank you for reporting them responsibly.

## Reporting a problem

Please **don't open a public issue**. Report it privately through GitHub:
[report a vulnerability](https://github.com/manoj1604/odi/security/advisories/new).
Include what's affected, how to reproduce it, and what an attacker could do.

You'll get an answer within a week. We'll agree on a fix and a disclosure
date with you, and credit you in the release notes unless you'd rather not
be named.

## Supported versions

Before 1.0, only the latest release gets security fixes.

## What's in scope

Especially welcome:

- An ODI file that makes a reader read or write outside what it should
  (paths leaving the file, links, resource exhaustion), or that passes the
  checks with contents that don't match its digest.
- Signature checks that accept what they shouldn't.
- The registry client trusting what a registry sends without checking it.
- The browser runner serving unchecked bytes, or crossing origins.

Not in scope: a program you chose to run doing what programs can do. odi
checks that a file is what its digest and signatures say; it isn't a
sandbox. Run untrusted programs in a VM or container.
