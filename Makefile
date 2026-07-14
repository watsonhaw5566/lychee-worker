# Default Makefile for lychee_worker (PHP extension built with Rust/cargo).
#
# This Makefile exists so that:
#   1) `make` / `make install` works out-of-the-box without running `./configure` first.
#   2) PIE (PHP Installer for Extensions) can drive the standard build pipeline.
#
# If you want to re-generate this Makefile (e.g. with custom configure options),
# run `./configure` from the project root.

SHELL := /bin/bash
PROJECT_DIR := $(shell dirname $(realpath $(lastword $(MAKEFILE_LIST))))

.PHONY: all install clean

all:
	cd "$(PROJECT_DIR)" && cargo build --release

install: all
	cd "$(PROJECT_DIR)" && bash scripts/install.sh

clean:
	cd "$(PROJECT_DIR)" && cargo clean