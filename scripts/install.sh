#!/usr/bin/env bash
#
# install.sh — copy the built lychee_worker extension into PHP's extension
# directory and (optionally) enable it in php.ini.
#
# Called by PIE after `cargo build --release`. Can also be invoked manually:
#
#   bash scripts/install.sh
#   bash scripts/install.sh --no-ini     # only copy .so, skip php.ini edit
#   bash scripts/install.sh --ini=/path/to/custom/php.ini

set -euo pipefail

EXT_NAME="lychee_worker"
GITHUB_REPO="watsonhaw/lychee-worker"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# --- detect OS -----------------------------------------------------------
# Extension filename in PHP extension dir keeps the platform native suffix so that the
# dynamic linker / code-signing mechanisms on macOS handle it correctly.
#   - Linux -> lychee_worker.so
#   - macOS -> lychee_worker.dylib
DETECTED_OS="$(uname -s 2>/dev/null || echo Unknown)"
case "${DETECTED_OS}" in
    Linux*)     EXT_FILENAME_EXT="so" ;;
    Darwin*)    EXT_FILENAME_EXT="dylib" ;;
    *)          EXT_FILENAME_EXT="so" ;;
esac
EXT_FILENAME="${EXT_NAME}.${EXT_FILENAME_EXT}"

# --- resolve arguments --------------------------------------------------------
INI_PATH=""
WRITE_INI=1
RELEASE_TAG=""          # non-empty => download prebuilt from GitHub Release, skip cargo build
for arg in "$@"; do
    case "${arg}" in
        --no-ini)
            WRITE_INI=0
            ;;
        --ini=*)
            INI_PATH="${arg#--ini=}"
            ;;
        --from-github-release=*)
            RELEASE_TAG="${arg#--from-github-release=}"
            ;;
        -h|--help)
            echo "Usage: $(basename "$0") [--no-ini] [--ini=/path/to/custom/php.ini] [--from-github-release=<tag>]"
            echo ""
            echo "  --no-ini                    skip writing php.ini / conf.d"
            echo "  --ini=PATH                  write to a custom php.ini path"
            echo "  --from-github-release=TAG   download a prebuilt extension binary from"
            echo "                              GitHub Release (e.g. '0.1.0' or 'latest')"
            echo "                              instead of building from source with cargo"
            exit 0
            ;;
        *)
            echo "[install] unknown argument: ${arg}" >&2
            exit 1
            ;;
    esac
done

# --- check tooling ------------------------------------------------------------
if ! command -v php-config >/dev/null 2>&1; then
    echo "[install] php-config not found in PATH — please install php-dev (or equivalent) first." >&2
    exit 1
fi

if [ -z "${RELEASE_TAG}" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "[install] cargo not found in PATH — please install Rust toolchain, or use --from-github-release=<tag> to install a prebuilt binary." >&2
        exit 1
    fi
fi

# --- helper: download a prebuilt extension from a GitHub Release --------
download_prebuilt() {
    local tag="$1"
    local dest="$2"
    local downloader=""
    if command -v curl >/dev/null 2>&1; then
        downloader="curl"
    elif command -v wget >/dev/null 2>&1; then
        downloader="wget"
    else
        echo "[install] neither curl nor wget found in PATH — cannot download prebuilt binary." >&2
        return 1
    fi

    # Resolve 'latest' -> actual release tag
    local resolved_tag="$tag"
    if [ "$tag" = "latest" ]; then
        echo "[install] resolving 'latest' release tag..."
        local latest_api="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
        if [ "$downloader" = "curl" ]; then
            resolved_tag="$(curl -fsSL "$latest_api" 2>/dev/null | grep -oP '(?<="tag_name": ")[^"]+' | head -n1 || true)"
        else
            resolved_tag="$(wget -qO- "$latest_api" 2>/dev/null | grep -oP '(?<="tag_name": ")[^"]+' | head -n1 || true)"
        fi
        if [ -z "$resolved_tag" ]; then
            echo "[install] failed to resolve 'latest' tag — please specify an explicit version, e.g. '0.1.0'." >&2
            return 1
        fi
        echo "[install] resolved to tag: $resolved_tag"
    fi

    # Figure out host OS / arch / libc / PHP version to match PIE naming.
    local php_version arch os_tag libc_tag
    php_version="$(php-config --version | cut -d. -f1,2)"
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="arm64" ;;
    esac
    if [ "$DETECTED_OS" = "Darwin" ]; then
        os_tag="Darwin"; libc_tag="bsdlibc"
    else
        os_tag="Linux"
        if ldd --version 2>/dev/null | grep -qi musl; then
            libc_tag="musl"
        else
            libc_tag="glibc"
        fi
    fi
    local pie_name="php_${EXT_NAME}-${resolved_tag}_php${php_version}-${arch}-${os_tag}-${libc_tag}-release-nts.zip"
    local legacy_name="liblychee_worker-${os_tag}.so"

    local release_api="https://api.github.com/repos/${GITHUB_REPO}/releases/tags/${resolved_tag}"
    echo "[install] querying ${release_api} for prebuilt assets..."
    local json_body=""
    if [ "$downloader" = "curl" ]; then
        json_body="$(curl -fsSL "$release_api" 2>/dev/null || true)"
    else
        json_body="$(wget -qO- "$release_api" 2>/dev/null || true)"
    fi
    if [ -z "$json_body" ]; then
        echo "[install] failed to fetch release metadata for tag '${resolved_tag}'." >&2
        return 1
    fi

    # Parse out 'name' lines and 'browser_download_url' lines, then match them.
    local names urls
    names="$(echo "$json_body" | grep -oP '"name":\s*"\K[^"]+' || true)"
    urls="$(echo "$json_body"  | grep -oP '"browser_download_url":\s*"\K[^"]+' || true)"

    local matched_name=""
    for candidate in "$pie_name" "$legacy_name"; do
        if echo "$names" | grep -Fxq "$candidate"; then
            matched_name="$candidate"
            break
        fi
    done
    if [ -z "$matched_name" ]; then
        echo "[install] no prebuilt asset matched this host (OS=${DETECTED_OS}, arch=${arch}, php=${php_version})." >&2
        echo "[install] candidates tried: ${pie_name}, ${legacy_name}" >&2
        echo "[install] please build from source by running this script without --from-github-release." >&2
        return 1
    fi
    local asset_url
    asset_url="$(echo "$urls" | grep -F "/${matched_name}" | head -n1 || true)"
    if [ -z "$asset_url" ]; then
        echo "[install] found asset name '${matched_name}' but could not locate its browser_download_url." >&2
        return 1
    fi

    echo "[install] downloading prebuilt binary: ${asset_url}"
    local download_path="${dest}.download"
    if [ "$downloader" = "curl" ]; then
        curl -fsSL -o "$download_path" "$asset_url" || return 1
    else
        wget -qO "$download_path" "$asset_url" || return 1
    fi

    # Detect whether we got a ZIP archive (PIE format) or a raw .so
    local mime="application/octet-stream"
    if command -v file >/dev/null 2>&1; then
        mime="$(file -b --mime-type "$download_path" 2>/dev/null || echo application/octet-stream)"
    else
        case "$matched_name" in
            *.zip) mime="application/zip" ;;
        esac
    fi

    if [ "$mime" = "application/zip" ]; then
        local extract_dir
        extract_dir="$(mktemp -d 2>/dev/null || mktemp -d -t lychee_worker_install)"
        if ! command -v unzip >/dev/null 2>&1; then
            echo "[install] downloaded a ZIP archive but 'unzip' is not installed — please install unzip." >&2
            rm -rf "$extract_dir"; rm -f "$download_path"
            return 1
        fi
        unzip -q -d "$extract_dir" "$download_path" || { rm -rf "$extract_dir"; rm -f "$download_path"; return 1; }
        local extracted
        extracted="$(find "$extract_dir" -type f \( -name "${EXT_NAME}.so" -o -name "lib${EXT_NAME}.so" -o -name "${EXT_NAME}.dylib" -o -name "lib${EXT_NAME}.dylib" \) | head -n1 || true)"
        if [ -z "$extracted" ]; then
            echo "[install] ZIP archive does not contain a recognizable ${EXT_NAME} binary." >&2
            rm -rf "$extract_dir"; rm -f "$download_path"
            return 1
        fi
        cp -f "$extracted" "$dest"
        rm -rf "$extract_dir"
    else
        cp -f "$download_path" "$dest"
    fi
    rm -f "$download_path"
    return 0
}

# --- resolve artifact: prebuilt download, or find/build from source ----
BUILD_ARTIFACT_EXT="${EXT_FILENAME_EXT}"
BUILD_ARTIFACT="${PROJECT_DIR}/target/release/lib${EXT_NAME}.${BUILD_ARTIFACT_EXT}"

if [ -n "${RELEASE_TAG}" ]; then
    # Skip cargo entirely; drop the downloaded file into target/release/ so the
    # rest of the script (copy / codesign / php.ini injection) flows unchanged.
    mkdir -p "${PROJECT_DIR}/target/release"
    if ! download_prebuilt "${RELEASE_TAG}" "${BUILD_ARTIFACT}"; then
        echo "[install] prebuilt download failed." >&2
        exit 1
    fi
else
    # Fallback: if the expected artifact doesn't exist, try other common extensions
    # (useful when the script runs on a host different from the build host, or when
    # cargo config overrides the default naming).
    if [ ! -f "${BUILD_ARTIFACT}" ]; then
        for _ext in so dylib; do
            _candidate="${PROJECT_DIR}/target/release/lib${EXT_NAME}.${_ext}"
            if [ -f "${_candidate}" ]; then
                BUILD_ARTIFACT="${_candidate}"
                BUILD_ARTIFACT_EXT="${_ext}"
                echo "[install] detected build artifact with non-default extension: ${_ext}"
                break
            fi
        done
    fi

    # --- ensure the build artifact exists, compile if not ------------------
    if [ ! -f "${BUILD_ARTIFACT}" ]; then
        echo "[install] no build artifact found at ${BUILD_ARTIFACT} — building release target now (OS=${DETECTED_OS})..."
        if ! (cd "${PROJECT_DIR}" && cargo build --release); then
            echo "[install] cargo build --release failed on ${DETECTED_OS}. Please check the Rust compilation output above." >&2
            echo "[install] common causes: missing Rust toolchain (run 'rustup update stable'), missing C toolchain (macOS: 'xcode-select --install'), or PHP headers missing ('php-config --includes')." >&2
            echo "[install] alternative: rerun with --from-github-release=<tag> to install a prebuilt binary without cargo." >&2
            exit 1
        fi
        # after build, re-detect using the native extension for this OS
        BUILD_ARTIFACT="${PROJECT_DIR}/target/release/lib${EXT_NAME}.${EXT_FILENAME_EXT}"
        if [ ! -f "${BUILD_ARTIFACT}" ]; then
            # last resort: find any liblychee_worker.* under target/release
            _found="$(find "${PROJECT_DIR}/target/release" -maxdepth 1 -type f -name "lib${EXT_NAME}.*" 2>/dev/null | head -n 1)"
            if [ -n "${_found}" ]; then
                BUILD_ARTIFACT="${_found}"
            else
                echo "[install] cargo build succeeded but no lib${EXT_NAME}.* artifact found under ${PROJECT_DIR}/target/release/" >&2
                echo "[install] check that Cargo.toml's [lib] section has crate-type = [\"cdylib\"] and that there are no build warnings above." >&2
                exit 1
            fi
        fi
    fi
fi

EXTENSION_DIR="$(php-config --extension-dir)"
DEST_PATH="${EXTENSION_DIR}/${EXT_FILENAME}"

# --- copy into PHP's extension dir ---------------------------------------
echo "[install] copying ${BUILD_ARTIFACT} -> ${DEST_PATH}"
if [ ! -w "${EXTENSION_DIR}" ]; then
    echo "[install] ${EXTENSION_DIR} not writable — retrying with sudo..." >&2
    sudo cp "${BUILD_ARTIFACT}" "${DEST_PATH}"
else
    cp "${BUILD_ARTIFACT}" "${DEST_PATH}"
fi

# --- macOS: ad-hoc code sign so that SIP accepts the extension ----------
if [ "${DETECTED_OS}" = "Darwin" ]; then
    if command -v codesign >/dev/null 2>&1; then
        echo "[install] ad-hoc signing ${DEST_PATH} for macOS Gatekeeper / SIP compatibility..."
        if ! codesign --force --deep -s - "${DEST_PATH}" 2>/dev/null; then
            # Retry with sudo if the target dir is not user-writable
            sudo codesign --force --deep -s - "${DEST_PATH}"
        fi
    else
        echo "[install] warning: 'codesign' not found in PATH — extension may fail to load on macOS due to SIP." >&2
    fi
fi

# --- enable in php.ini (unless --no-ini) ---------------------------------
# Use the absolute path to the shared library. This makes the loading
# independent of PHP's extension_dir search path and avoids suffix-related
# confusion on macOS.
INI_LINE="extension=${DEST_PATH}"
if [ "${WRITE_INI}" -eq 1 ]; then
    if [ -z "${INI_PATH}" ]; then
        INI_PATH="$(php --ini | awk -F': ' '/Loaded Configuration File/ {print $2; exit}')"
        if [ -z "${INI_PATH}" ] || [ ! -f "${INI_PATH}" ]; then
            CONF_DIR="$(php-config --ini-dir 2>/dev/null || true)"
            if [ -n "${CONF_DIR}" ] && [ -d "${CONF_DIR}" ]; then
                INI_PATH="${CONF_DIR}/99-${EXT_NAME}.ini"
                echo "[install] no loaded php.ini detected — will create scan-dir snippet: ${INI_PATH}"
            else
                echo "[install] could not determine php.ini path — skipping ini injection." >&2
                echo "[install] please manually add: ${INI_LINE}" >&2
                INI_PATH=""
            fi
        fi
    fi

    if [ -n "${INI_PATH}" ]; then
        # Before writing, check if any scan-dir snippet or the php.ini already
        # references this extension (under any name) — otherwise we get
        # "Module 'lychee_worker' is already loaded" warnings on every run.
        _already_registered=0
        _conf_dir_for_check="$(php-config --ini-dir 2>/dev/null || true)"
        if [ -n "${_conf_dir_for_check}" ] && [ -d "${_conf_dir_for_check}" ]; then
            if grep -lRq "extension.*${EXT_NAME}" "${_conf_dir_for_check}/" 2>/dev/null; then
                _already_registered=1
            fi
        fi
        if [ -f "${INI_PATH}" ] && grep -qF "${INI_LINE}" "${INI_PATH}"; then
            _already_registered=1
        fi

        if [ "${_already_registered}" -eq 1 ]; then
            echo "[install] extension '${EXT_NAME}' is already registered in PHP's ini — skipping duplicate injection."
        else
            if [ ! -w "${INI_PATH}" ] && [ -f "${INI_PATH}" ]; then
                echo "[install] ${INI_PATH} not writable — retrying with sudo..." >&2
                echo "${INI_LINE}" | sudo tee -a "${INI_PATH}" >/dev/null
            else
                echo "${INI_LINE}" >> "${INI_PATH}"
            fi
            echo "[install] appended '${INI_LINE}' to ${INI_PATH}"
        fi
    fi
else
    echo "[install] --no-ini set — skipping php.ini injection."
fi

# --- verify -------------------------------------------------------------
if command -v php >/dev/null 2>&1; then
    # conf.d 扫描已经注入的 ini 已经让 PHP 启动时加载扩展，这里只需要用 php -m 验证已被加载即可；
    # 不要用 -d extension=... 再次注入，否则会触发 "already loaded" 警告。
    if php -m 2>/dev/null | grep -q "^${EXT_NAME}$"; then
        echo "[install] OK — extension '${EXT_NAME}' loaded."
    else
        # 如果 conf.d 还没生效（或 ini 没有被扫描），给出明确诊断。
        echo "[install] extension installed, but 'php -m' does not yet show '${EXT_NAME}'." >&2
        echo "[install] diagnostic — attempting explicit load to surface the error:" >&2
        php -d "extension=${DEST_PATH}" -m 2>&1 | head -n 20 >&2 || true
        echo "[install] common fixes: check codesign output above, verify PHP version matches build-time php-config, or restart your PHP SAPI." >&2
    fi
fi