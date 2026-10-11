# The SSH login shell execs this wrapper: $$ must remain the owned group leader.
set -eu
umask 077
owner=$$
owned=0
cache_root=${XDG_CACHE_HOME:-"$HOME/.cache"}/herdr-gpui/vscode
runtime=
job=
watcher=
cleanup() {
    set +e
    trap '' HUP INT TERM PIPE
    trap - 0
    [ "$owned" = 1 ] || exit 76
    kill -TERM "-$owner" 2>/dev/null
    sleep "$grace"
    if [ -n "$job" ] && ! kill -0 "$job" 2>/dev/null; then wait "$job" 2>/dev/null; fi
    if [ -n "$watcher" ] && ! kill -0 "$watcher" 2>/dev/null; then wait "$watcher" 2>/dev/null; fi
    if [ -n "$runtime" ]; then
        if [ -f "$runtime/cache-staging" ]; then
            IFS= read -r staging < "$runtime/cache-staging"
            suffix=${staging#"$cache_root"/.install.}
            case "$suffix" in ''|*[!a-zA-Z0-9]*) ;; *)
                if [ "$staging" = "$cache_root/.install.$suffix" ] && [ "${#suffix}" = 8 ]; then rm -rf -- "$staging"; fi;;
            esac
        fi
        rm -rf -- "$runtime"
    fi
    kill -KILL "-$owner"
}
trap cleanup 0
trap cleanup HUP INT TERM PIPE
pgid=$(ps -o pgid= -p "$owner")
set -- $pgid
[ "$#" = 1 ] && [ "$1" = "$owner" ] || exit 76
owned=1
IFS=' ' read -r record received token extra || exit 76
[ "$record" = token ] && [ "$received" = "$generation" ] && [ -z "$extra" ] || exit 76
case "$token" in ''|*[!a-zA-Z0-9_-]*) exit 76;; esac
[ "${#token}" -le 256 ] || exit 76
base=${XDG_STATE_HOME:-"$HOME/.local/state"}/herdr-gpui
mkdir -p "$base"
runtime=$(mktemp -d "$base/code-runtime.XXXXXXXX")
mkdir "$runtime/heartbeat"
printf '%s' "$token" > "$runtime/token"
unset token
chmod 600 "$runtime/token"
fail() {
    printf 'FAIL %s %s\n' "$generation" "$1" || :
    kill -TERM "$owner"
    exit 1
}
launch() {
    # The watchdog and traps already own all bootstrap descendants.
    cli=
    reader=
    trap 'trap "" HUP INT TERM; for child in "$cli" "$reader"; do if [ -n "$child" ]; then kill "$child" 2>/dev/null || :; wait "$child" 2>/dev/null || :; fi; done; exit 0' HUP INT TERM
    bootstrap
    data=$base/code-data/$target_key
    prepare_private_directory "$data" || fail 2
    printf 'START %s 0\n' "$generation" || fail 3
    mkfifo "$runtime/output-pipe"
    (
        trap 'exit 0' HUP INT TERM
        dd bs=1 count=65536 of="$runtime/output" 2>/dev/null
        : > "$runtime/capture-full"
        exec cat >/dev/null
    ) < "$runtime/output-pipe" & reader=$!
    "$program" serve-web --host 127.0.0.1 --port 0 --connection-token-file "$runtime/token" --server-data-dir "$data" --accept-server-license-terms > "$runtime/output-pipe" 2>/dev/null &
    cli=$!
    wait "$cli" || :
    : > "$runtime/exited"
}
watch() {
    sleeper=
    trap 'if [ -n "$sleeper" ]; then kill "$sleeper" 2>/dev/null || :; wait "$sleeper" 2>/dev/null || :; fi; exit 0' HUP INT TERM
    trap '' PIPE
    count=0
    announced=0
    while :; do
        sleep 1 & sleeper=$!
        wait "$sleeper" || exit 0
        sleeper=
        if [ -f "$runtime/exited" ]; then fail 4; fi
        if [ "$announced" = 0 ] && [ -f "$runtime/capture-full" ] && [ "$(wc -c < "$runtime/output")" -ge 65536 ]; then fail 3; fi
        if [ "$announced" = 0 ] && [ -f "$runtime/output" ]; then
            # No token-bearing CLI output ever leaves this private file.
            port=$(sed -n 's/^.*Web UI available at http:\/\/127\.0\.0\.1:\([0-9][0-9]*\).*$/\1/p' "$runtime/output" | head -n 1)
            case "$port" in ''|*[!0-9]*) ;; *)
                [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail 3
                printf 'READY %s %s\n' "$generation" "$port" || { kill -TERM "$owner"; exit 1; }
                announced=1;;
            esac
        fi
        count=$((count + 1))
        if [ "$count" -ge "$lease" ]; then
            count=0
            rmdir "$runtime/heartbeat" 2>/dev/null || { kill -TERM "$owner"; exit 0; }
        fi
    done
}
watch & watcher=$!
launch & job=$!
while IFS=' ' read -r record received extra; do
    [ "$received" = "$generation" ] && [ -z "$extra" ] || continue
    case "$record" in
        ping) mkdir "$runtime/heartbeat" 2>/dev/null || :;;
        stop) break;;
    esac
done
cleanup
