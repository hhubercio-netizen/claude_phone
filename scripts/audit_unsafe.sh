#!/usr/bin/env bash
# TM-CODE.2 — list every `unsafe` block in the workspace.
#
# This is a developer / reviewer convenience, NOT a CI gate. The actual
# enforcement is the `#![deny(unsafe_code)]` crate-level attribute on each
# crate root; the compiler refuses to build any new `unsafe` that lands
# outside the `#![allow(unsafe_code)]` module (currently `tty.rs`).
#
# Filters out the false-positive token `unsafe-inline` inside Content-
# Security-Policy header strings, and any `unsafe` appearing inside a `//`
# or `///` comment.
set -euo pipefail

rg --no-heading -n '\bunsafe\b' crates/*/src \
    | grep -v 'unsafe-inline' \
    | grep -v '^\s*//' \
    | grep -v '^\s*///'
