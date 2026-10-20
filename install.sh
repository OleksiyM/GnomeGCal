#!/usr/bin/env bash
# GnomeGCal per-user install/update from a signed release archive.
#   bash install.sh [--version v1.0.0] [--require-provenance]
# Designed to be run safely via: curl -fsSL https://raw.githubusercontent.com/OleksiyM/GnomeGCal/main/install.sh | bash
gcal_install() (
    set -euo pipefail
    fail() { printf 'Install failed: %s\n' "$*" >&2; exit 1; }
    trap 'printf "Installation did not complete. See the error above; fix it and rerun.\n" >&2' ERR

    repo=OleksiyM/GnomeGCal
    app_id=io.github.OleksiyM.GnomeGCal
    app_dir=${GNOME_GCAL_APP_DIR:-"$HOME/Applications/GnomeGCal"}
    data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
    config_dir=${XDG_CONFIG_HOME:-"$HOME/.config"}
    tag=''
    require_provenance=no
    while (($#)); do
        case "$1" in
            --version)
                [[ $# -ge 2 ]] || fail '--version needs a tag such as v1.0.0'
                tag=$2; shift 2 ;;
            --require-provenance) require_provenance=yes; shift ;;
            -h|--help)
                printf '%s\n' 'Install/update: bash install.sh [--version v1.0.0] [--require-provenance]' \
                    'Downloads the release archive, verifies SHA-256 (and provenance when gh is available),' \
                    'installs to ~/Applications/GnomeGCal (override: GNOME_GCAL_APP_DIR)' \
                    'and configures desktop launcher. No sudo, no root required.'
                exit 0 ;;
            *) fail "Unknown option: $1" ;;
        esac
    done

    printf '%s\n' '========================================================' \
        '  GnomeGCal (Installer / Updater)' \
        '========================================================'
    [[ $EUID -ne 0 ]] || fail 'Run as your desktop user, not with sudo.'
    for directory in "$app_dir" "$data_dir" "$config_dir"; do
        [[ $directory == /* && $directory != / ]] || fail 'Installation and XDG paths must be absolute, non-root paths.'
    done
    # shellcheck disable=SC1003
    case "$app_dir" in
        *['"$`%\']*|*'|'*|*'&'*|*$'\n'*|*$'\r'*) fail 'Installation path contains characters unsupported in desktop launchers.' ;;
    esac
    for tool in curl tar gzip sha256sum awk sed install mktemp uname; do
        command -v "$tool" >/dev/null || fail "Missing required command: $tool"
    done
    case "$(uname -m)" in
        x86_64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail 'Only x86_64 and aarch64 release archives are supported.' ;;
    esac

    download=(curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 300)
    if [[ -z "$tag" ]]; then
        latest=$("${download[@]}" --head --output /dev/null --write-out '%{url_effective}' "https://github.com/$repo/releases/latest")
        [[ $latest == "https://github.com/$repo/releases/tag/"* ]] || fail 'No stable release found.'
        tag=${latest##*/}
    fi
    [[ $tag =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || fail 'Expected a stable tag such as v1.0.0.'
    package="gnome-gcal-${tag#v}-$arch"
    archive="$package.tar.gz"
    url="https://github.com/$repo/releases/download/$tag"
    temporary=$(mktemp -d /tmp/gnome-gcal-install.XXXXXXXX)
    trap 'rm -rf -- "$temporary"' EXIT

    printf '\n📦 GnomeGCal %s · %s\n\n⬇️  Downloading release…\n' "$tag" "$arch"
    "${download[@]}" --output "$temporary/$archive" "$url/$archive"
    "${download[@]}" --output "$temporary/SHA256SUMS" "$url/SHA256SUMS"

    printf '\n🔒 Verifying SHA-256…\n'
    hash=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$temporary/SHA256SUMS")
    [[ $hash =~ ^[0-9a-fA-F]{64}$ ]] || fail 'Missing or duplicate archive checksum.'
    (cd "$temporary"; printf '%s  %s\n' "$hash" "$archive" | sha256sum --check --status -) || fail 'SHA-256 mismatch. Nothing installed.'
    printf '✅ SHA-256 verified.\n'

    verify=no
    if command -v gh >/dev/null && help=$(gh attestation verify --help 2>/dev/null); then
        verify=yes
        for option in --bundle --repo --signer-workflow --source-ref --cert-oidc-issuer --deny-self-hosted-runners --predicate-type; do
            [[ $help == *"$option"* ]] || verify=no
        done
    fi
    if [[ $verify == yes ]]; then
        printf '\n🛡️  Verifying release provenance…\n'
        "${download[@]}" --output "$temporary/$archive.sigstore.json" "$url/$archive.sigstore.json"
        gh attestation verify "$temporary/$archive" \
            --bundle "$temporary/$archive.sigstore.json" --repo "$repo" \
            --signer-workflow "$repo/.github/workflows/release.yml" --source-ref "refs/tags/$tag" \
            --cert-oidc-issuer https://token.actions.githubusercontent.com \
            --deny-self-hosted-runners --predicate-type https://slsa.dev/provenance/v1
        printf '✅ Provenance verified.\n'
    elif [[ $require_provenance == yes ]]; then
        fail '--require-provenance needs gh with the required attestation flags. Install or update gh from a trusted source.'
    else
        printf '%s\n' '' 'ℹ️  Provenance skipped: gh is missing or too old. SHA-256 integrity passed.'
    fi

    printf '\n📂 Installing to %s…\n' "$app_dir"
    mkdir -p "$app_dir"
    tar -xzf "$temporary/$archive" -C "$app_dir" --strip-components=1

    printf '🎨 Setting up desktop integration…\n'
    mkdir -p "$data_dir/applications" "$data_dir/icons/hicolor/scalable/apps"
    if [[ -f "$app_dir/assets/google-calendar.svg" ]]; then
        install -m 0644 "$app_dir/assets/google-calendar.svg" "$data_dir/icons/hicolor/scalable/apps/$app_id.svg"
    fi

    sed -e "s|^Exec=.*|Exec=$app_dir/gnome-gcal|" \
        -e "s|^Icon=.*|Icon=$app_id|" \
        "$app_dir/data/$app_id.desktop.in" > "$data_dir/applications/$app_id.desktop"
    chmod 0644 "$data_dir/applications/$app_id.desktop"

    if command -v update-desktop-database >/dev/null; then
        update-desktop-database "$data_dir/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null; then
        gtk-update-icon-cache -q -t "$data_dir/icons/hicolor" 2>/dev/null || true
    fi

    printf '%s\n' '' \
        '🎉 GnomeGCal is installed and ready to launch!' \
        '💡 Search for "GnomeGCal" in your GNOME App Grid or run:' \
        "   $app_dir/gnome-gcal &"
)
gcal_install "$@"
