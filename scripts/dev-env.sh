#!/usr/bin/env bash

# Source this file to make the project-managed user toolchains available in the
# current shell. It is intentionally side-effect free.

webtop_manager_prepend_path() {
  case ":${PATH}:" in
    *":$1:"*) ;;
    *) PATH="$1:$PATH" ;;
  esac
}

webtop_manager_prepend_path "${HOME}/.local/bin"
webtop_manager_prepend_path "${HOME}/.cargo/bin"
export PATH
export WEBTOP_MANAGER_PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

unset -f webtop_manager_prepend_path
