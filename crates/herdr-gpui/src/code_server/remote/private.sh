# Both the read-only probe and bootstrap reject unsafe cache entries before reuse.
private_path() (
    path=$1
    expected=$2
    kind=$3
    [ ! -L "$path" ] || return 1
    case "$kind" in
        directory) [ -d "$path" ] || return 1;;
        file) [ -f "$path" ] || return 1;;
        *) return 1;;
    esac
    fields=$(stat -c '%u %a %h' "$path" 2>/dev/null) || fields=$(stat -f '%u %Lp %l' "$path" 2>/dev/null) || return 1
    set -- $fields
    [ "$#" = 3 ] && [ "$1" = "$(id -u)" ] && [ "$2" = "$expected" ] || return 1
    [ "$kind" = directory ] || [ "$3" = 1 ]
)
prepare_private_directory() {
    if [ ! -e "$1" ] && [ ! -L "$1" ]; then mkdir -p "$1" || return 1; fi
    private_path "$1" 700 directory
}
