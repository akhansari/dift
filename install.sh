#!/bin/sh

set -eu

LATEST_RELEASE_API='https://gitlab.com/api/v4/projects/akhansari%2Fdift/releases/permalink/latest'
RELEASE_DOWNLOAD_BASE='https://gitlab.com/akhansari/dift/-/releases'

say() {
    printf '%s\n' "$*"
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

has_command() {
    command -v "$1" >/dev/null 2>&1
}

fetch() {
    case "$downloader" in
    curl)
        curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location "$1"
        ;;
    wget)
        wget --quiet --output-document=- "$1"
        ;;
    esac
}

download() {
    case "$downloader" in
    curl)
        curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
            --output "$2" "$1"
        ;;
    wget)
        wget --quiet --output-document="$2" "$1"
        ;;
    esac
}

is_wsl() {
    [ -n "${WSL_INTEROP:-}" ] ||
        [ -n "${WSL_DISTRO_NAME:-}" ] ||
        case "$(uname -r)" in
        *[Mm]icrosoft*) return 0 ;;
        *) return 1 ;;
        esac
}

linux_libc() {
    if has_command getconf && getconf GNU_LIBC_VERSION >/dev/null 2>&1; then
        printf '%s\n' gnu
        return
    fi

    if has_command ldd; then
        ldd_version=$(ldd --version 2>&1 || :)
        case "$ldd_version" in
        *musl* | *MUSL*)
            printf '%s\n' musl
            return
            ;;
        *GLIBC* | *glibc* | *'GNU libc'*)
            printf '%s\n' gnu
            return
            ;;
        esac
    fi

    # The static musl build is the safest fallback when libc cannot be identified.
    printf '%s\n' musl
}

detect_platform() {
    os=$(uname -s)
    architecture=$(uname -m)

    case "$os" in
    Linux)
        install_platform=unix
        if is_wsl; then
            say 'Detected WSL; installing the Linux binary.'
        fi

        case "$architecture" in
        x86_64 | amd64)
            target="x86_64-unknown-linux-$(linux_libc)"
            ;;
        aarch64 | arm64)
            target='aarch64-unknown-linux-musl'
            ;;
        *)
            die "unsupported Linux architecture: $architecture"
            ;;
        esac
        binary_name=dift
        ;;
    MINGW* | MSYS* | CYGWIN*)
        install_platform=windows
        case "$architecture" in
        x86_64 | amd64) target='x86_64-pc-windows-gnu' ;;
        *) die "unsupported Windows architecture: $architecture" ;;
        esac
        binary_name=dift.exe
        ;;
    Darwin)
        install_platform=unix
        case "$architecture" in
        x86_64 | amd64) target='x86_64-apple-darwin' ;;
        aarch64 | arm64) target='aarch64-apple-darwin' ;;
        *) die "unsupported macOS architecture: $architecture" ;;
        esac
        binary_name=dift
        ;;
    *)
        die "unsupported operating system: $os"
        ;;
    esac
}

windows_path_to_shell() {
    has_command cygpath || die 'cygpath is required to resolve Windows install paths'
    cygpath --unix "$1"
}

shell_user_program_files() {
    if has_command powershell.exe; then
        powershell_command=powershell.exe
    elif has_command pwsh.exe; then
        powershell_command=pwsh.exe
    elif has_command powershell; then
        powershell_command=powershell
    elif has_command pwsh; then
        powershell_command=pwsh
    else
        return 1
    fi

    if ! program_files=$(
        "$powershell_command" -NoLogo -NoProfile -NonInteractive -Command \
            "(New-Object -ComObject Shell.Application).NameSpace('shell:UserProgramFiles').Self.Path" \
            2>/dev/null
    ); then
        return 1
    fi

    program_files=$(printf '%s' "$program_files" | tr -d '\r')
    [ -n "$program_files" ] || return 1
    windows_path_to_shell "$program_files"
}

resolve_install_dir() {
    if [ -n "${INSTALL_DIR:-}" ]; then
        install_dir=$INSTALL_DIR
    elif [ -n "${XDG_BIN_HOME:-}" ]; then
        install_dir=$XDG_BIN_HOME
    elif [ "$install_platform" = windows ]; then
        if install_dir=$(shell_user_program_files); then
            :
        elif [ -n "${LOCALAPPDATA:-}" ]; then
            local_app_data=$(windows_path_to_shell "$LOCALAPPDATA")
            install_dir="$local_app_data/Programs"
        else
            die 'cannot determine the Windows install directory; set INSTALL_DIR or XDG_BIN_HOME'
        fi
    elif [ -n "${HOME:-}" ]; then
        install_dir="$HOME/.local/bin"
    else
        die 'cannot determine the install directory; set INSTALL_DIR or XDG_BIN_HOME'
    fi

    [ -n "$install_dir" ] || die 'install directory cannot be empty'
    if [ "$install_dir" != / ]; then
        install_dir=${install_dir%/}
    fi
}

sha256() {
    if has_command sha256sum; then
        checksum_output=$(sha256sum "$1") || return 1
        printf '%s\n' "$checksum_output" | sed 's/[[:space:]].*//'
    elif has_command shasum; then
        checksum_output=$(shasum -a 256 "$1") || return 1
        printf '%s\n' "$checksum_output" | sed 's/[[:space:]].*//'
    elif has_command openssl; then
        checksum_output=$(openssl dgst -sha256 "$1") || return 1
        printf '%s\n' "$checksum_output" | sed 's/.*[[:space:]]//'
    else
        return 1
    fi
}

is_on_path() {
    remaining_path=${PATH:-}
    while :; do
        path_entry=${remaining_path%%:*}
        if [ "$path_entry" = "$install_dir" ]; then
            return 0
        fi

        case "$remaining_path" in
        *:*) remaining_path=${remaining_path#*:} ;;
        *) break ;;
        esac
    done
    return 1
}

temporary_directory=
staged_file=

cleanup() {
    if [ -n "$staged_file" ]; then
        rm -f "$staged_file" || :
    fi
    if [ -n "$temporary_directory" ]; then
        rm -rf "$temporary_directory" || :
    fi
}

trap cleanup 0
trap 'exit 1' HUP INT TERM

if has_command curl; then
    downloader=curl
elif has_command wget; then
    downloader=wget
else
    die 'curl or wget is required to download dift'
fi

detect_platform
resolve_install_dir

say 'Finding the latest dift release...'
if ! release_json=$(fetch "$LATEST_RELEASE_API"); then
    die 'failed to retrieve the latest GitLab release'
fi

version=$(
    printf '%s\n' "$release_json" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
)
case "$version" in
v[0-9]*) ;;
*) die 'the latest GitLab release did not contain a valid version tag' ;;
esac
case "$version" in
*[!A-Za-z0-9._+-]*) die "the latest GitLab release contained an unsafe version tag: $version" ;;
esac

archive="dift-$version-$target.tar.gz"
download_base="$RELEASE_DOWNLOAD_BASE/$version/downloads"
temporary_directory=$(mktemp -d)
archive_path="$temporary_directory/$archive"
checksum_path="$archive_path.sha256"

say "Downloading dift $version for $target..."
download "$download_base/$archive" "$archive_path" || die "failed to download $archive"
download "$download_base/$archive.sha256" "$checksum_path" || die "failed to download $archive.sha256"

if ! read -r expected_checksum _ <"$checksum_path"; then
    die 'failed to read the release checksum'
fi
[ "${#expected_checksum}" -eq 64 ] || die 'the release checksum is not a SHA-256 digest'
case "$expected_checksum" in
*[!A-Fa-f0-9]*) die 'the release checksum is not a SHA-256 digest' ;;
esac

if ! actual_checksum=$(sha256 "$archive_path"); then
    die 'sha256sum, shasum, or openssl is required to verify the download'
fi
expected_checksum=$(printf '%s' "$expected_checksum" | tr 'A-F' 'a-f')
actual_checksum=$(printf '%s' "$actual_checksum" | tr 'A-F' 'a-f')
[ "$actual_checksum" = "$expected_checksum" ] || die "checksum verification failed for $archive"

tar -xzf "$archive_path" -C "$temporary_directory" "$binary_name" ||
    die "failed to extract $binary_name"
[ -f "$temporary_directory/$binary_name" ] || die "the release archive does not contain $binary_name"
chmod 755 "$temporary_directory/$binary_name"

mkdir -p "$install_dir" || die "failed to create install directory: $install_dir"
destination="$install_dir/$binary_name"
staged_file=$(mktemp "$install_dir/.dift-install.XXXXXX") ||
    die "failed to create a temporary file in $install_dir"
cp "$temporary_directory/$binary_name" "$staged_file" || die 'failed to stage the dift binary'
chmod 755 "$staged_file" || die 'failed to make the dift binary executable'
mv -f "$staged_file" "$destination" || die "failed to install dift to $destination"
staged_file=

say "Installed dift $version to $destination"
if ! is_on_path; then
    say "Add $install_dir to PATH to run dift from any shell."
fi
