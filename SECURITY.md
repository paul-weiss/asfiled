# Security Policy

asfiled handles only public data, but its ingestion client makes outbound
requests and its published datasets are consumed downstream — correctness and
supply-chain integrity are security surfaces.

## Reporting a vulnerability

Please use [GitHub private vulnerability reporting](https://github.com/paul-weiss/asfiled/security/advisories/new)
rather than a public issue. Reports are acknowledged within a week.

## Scope

- The `asfiled` crate and CLI
- The published Parquet datasets and their manifests
- CI and release workflows in this repository

Dependencies are audited continuously against the RustSec advisory database
(see the Security audit workflow).
