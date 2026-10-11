bootstrap() {
    candidate=$(command -v code 2>/dev/null || :)
    case "$candidate" in /*) ;; *) candidate=;; esac
    if [ -n "$candidate" ]; then
        sibling=${candidate%/*}/code-tunnel
        if [ -x "$sibling" ]; then candidate=$sibling; fi
        if "$candidate" --version 2>/dev/null | grep -Eq '[0-9a-f]{40}' && "$candidate" serve-web --help >/dev/null 2>&1; then program=$candidate; return; fi
    fi
    cache=${XDG_CACHE_HOME:-"$HOME/.cache"}/herdr-gpui/vscode
    prepare_private_directory "$cache" || fail 2
    pointer=$cache/current-$platform
    if [ -e "$pointer" ] || [ -L "$pointer" ]; then
        private_path "$pointer" 600 file || fail 2
        cached=$(head -c 42 "$pointer") || fail 2
        case "$cached" in *[!a-f0-9]*|'') ;; *)
            if [ "${#cached}" = 40 ]; then
                program=$cache/$cached-$platform
                if [ -e "$program" ] || [ -L "$program" ]; then
                    private_path "$program" 700 file || fail 2
                    if "$program" serve-web --help >/dev/null 2>&1; then return; fi
                fi
            fi;;
        esac
    fi
    [ -n "$artifact" ] || fail 2
    installed=$cache/$commit-$platform
    if [ -e "$installed" ] || [ -L "$installed" ]; then
        private_path "$installed" 700 file || fail 2
        if "$installed" serve-web --help >/dev/null 2>&1; then program=$installed; return; fi
    fi
    for tool in curl tar gzip head; do command -v "$tool" >/dev/null 2>&1 || fail 1; done
    if command -v sha256sum >/dev/null 2>&1; then digest=sha256sum
    elif command -v shasum >/dev/null 2>&1; then digest=shasum
    else fail 1; fi
    staging=$(mktemp -d "$cache/.install.XXXXXXXX")
    printf '%s\n' "$staging" > "$runtime/cache-staging"
    curl --fail --silent --show-error --proto '=https' --proto-redir '=https' --max-time 180 --max-filesize 67108864 "$artifact" -o "$staging/cli.tar.gz" 2>/dev/null || fail 5
    size=$(wc -c < "$staging/cli.tar.gz")
    [ "$size" -le 67108864 ] || fail 2
    if [ "$digest" = sha256sum ]; then actual=$(sha256sum "$staging/cli.tar.gz"); else actual=$(shasum -a 256 "$staging/cli.tar.gz"); fi
    set -- $actual
    [ "$1" = "$sha256" ] || fail 2
    # Extract only one regular executable named code, never links or paths.
    gzip -dc "$staging/cli.tar.gz" 2>/dev/null | head -c 67108865 > "$staging/cli.tar" || fail 2
    size=$(wc -c < "$staging/cli.tar")
    [ "$size" -le 67108864 ] || fail 2
    names=$(tar -tf "$staging/cli.tar" 2>/dev/null) || fail 2
    [ "$names" = code ] || fail 2
    listing=$(tar -tvf "$staging/cli.tar" 2>/dev/null) || fail 2
    case "$listing" in -??x*|-?????x*|-????????x*) ;; *) fail 2;; esac
    tar -xf "$staging/cli.tar" -C "$staging" code 2>/dev/null || fail 2
    [ -f "$staging/code" ] && [ ! -L "$staging/code" ] || fail 2
    chmod 700 "$staging/code"
    "$staging/code" --version 2>/dev/null | grep -F "$commit" >/dev/null || fail 2
    "$staging/code" serve-web --help >/dev/null 2>&1 || fail 2
    # Same-filesystem rename exposes only a complete, validated executable.
    mv -f "$staging/code" "$installed" || fail 2
    printf '%s\n' "$commit" > "$staging/current"
    mv -f "$staging/current" "$pointer" || fail 2
    rm -rf -- "$staging"
    rm "$runtime/cache-staging"
    program=$installed
}
