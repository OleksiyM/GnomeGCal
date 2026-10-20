#!/usr/bin/env bash
# GnomeGCal removal. Usage: bash uninstall.sh [--purge]
# Session cookies and settings (~/.config/gnome-gcal, ~/.local/share/gnome-gcal)
# are kept by default unless --purge is given.
gcal_uninstall() (
    set -euo pipefail
    fail() { printf 'Uninstall failed: %s\n' "$*" >&2; exit 1; }
    app_id=io.github.OleksiyM.GnomeGCal
    app_dir=${GNOME_GCAL_APP_DIR:-"$HOME/Applications/GnomeGCal"}
    data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
    config_dir=${XDG_CONFIG_HOME:-"$HOME/.config"}
    purge=no
    for arg in "$@"; do
        case "$arg" in
            --purge) purge=yes ;;
            -h|--help)
                printf '%s\n' 'Uninstall: bash uninstall.sh [--purge]' \
                    'Stops GnomeGCal and removes application launchers.' \
                    'Settings and login session are kept unless --purge is supplied.'
                exit 0 ;;
            *) fail "Unknown option: $arg" ;;
        esac
    done
    printf '%s\n' '' '📦 GnomeGCal · uninstall' ''
    [[ $EUID -ne 0 ]] || fail 'Run as your desktop user, not with sudo.'
    for directory in "$app_dir" "$data_dir" "$config_dir"; do
        [[ $directory == /* && $directory != / ]] || fail 'Installation and XDG paths must be absolute, non-root paths.'
    done

    printf '🛑 Stopping running GnomeGCal instances…\n'
    pkill -f 'gnome-gcal' 2>/dev/null || true

    printf '🗑️  Removing application files and desktop integration…\n'
    rm -f -- "$data_dir/applications/$app_id.desktop" \
        "$data_dir/icons/hicolor/scalable/apps/$app_id.svg" \
        "$app_dir/gnome-gcal" "$app_dir/uninstall.sh"
    rm -rf -- "$app_dir/data" "$app_dir/assets" 2>/dev/null || true
    rmdir -- "$app_dir" 2>/dev/null || true

    if [[ $purge == yes ]]; then
        printf '%s\n' '⚠️  Purging settings, cached data, and login sessions.'
        rm -rf -- "$config_dir/gnome-gcal" "$data_dir/gnome-gcal" "$HOME/.cache/gnome-gcal"
    else
        printf '%s\n' '📝 Settings and Google session cookies were kept (use --purge to remove them).'
    fi

    if command -v update-desktop-database >/dev/null; then
        update-desktop-database "$data_dir/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null; then
        gtk-update-icon-cache -q -t "$data_dir/icons/hicolor" 2>/dev/null || true
    fi
    printf '%s\n' '' '✅ GnomeGCal uninstalled successfully.'
)
gcal_uninstall "$@"
