#!/bin/sh
# Managed by prehook. Runs [tool.prehook].hooks from pyproject.toml.
# Self-contained: POSIX sh + awk, no prehook binary needed at commit time.
# Stage is this hook's filename (pre-commit, pre-push, ...), or $PREHOOK_STAGE
# when invoked by `prehook run`.

stage="${PREHOOK_STAGE:-$(basename "$0")}"
root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
config="$root/pyproject.toml"
[ -f "$config" ] || exit 0

SEP=$(printf '\034')

# Parse [tool.prehook]: emit one "verbose<SEP>name<SEP>cmd" line per hook that
# matches this stage, plus FAILFAST / PARALLEL marker lines for config flags.
records=$(awk -v stage="$stage" '
  BEGIN { sect=0; arr=0; depth=0; instr=0; buf="" }
  !arr && /^[[:space:]]*\[tool\.prehook\][[:space:]]*$/ { sect=1; next }
  !arr && /^[[:space:]]*\[/                             { sect=0; next }
  sect && !arr && /^[[:space:]]*fail_fast[[:space:]]*=[[:space:]]*true/ { print "FAILFAST"; next }
  sect && !arr && /^[[:space:]]*parallel[[:space:]]*=[[:space:]]*true/  { print "PARALLEL"; next }
  sect && !arr && /^[[:space:]]*hooks[[:space:]]*=/ { sub(/^[^=]*=/, "", $0); arr=1 }
  arr { scan($0); if (!arr) split_and_emit(); next }

  function scan(line,   i, ch) {
    for (i = 1; i <= length(line); i++) {
      ch = substr(line, i, 1)
      if (instr) { buf = buf ch; if (ch == "\"") instr = 0; continue }
      if (ch == "#")  return
      if (ch == "\"") { instr = 1; buf = buf ch; continue }
      if (ch == "[")  { depth++; if (depth > 1) buf = buf ch; continue }
      if (ch == "]")  { depth--; if (depth == 0) { arr = 0; return } buf = buf ch; continue }
      buf = buf ch
    }
    buf = buf " "
  }
  function split_and_emit(   i, ch, el, bd, ss) {
    el = ""; bd = 0; ss = 0
    for (i = 1; i <= length(buf); i++) {
      ch = substr(buf, i, 1)
      if (ss) { el = el ch; if (ch == "\"") ss = 0; continue }
      if (ch == "\"") { ss = 1; el = el ch; continue }
      if (ch == "{") { bd++; el = el ch; continue }
      if (ch == "}") { bd--; el = el ch; continue }
      if (ch == "," && bd == 0) { emit(el); el = ""; continue }
      el = el ch
    }
    emit(el)
  }
  function emit(el,   run, nm, on, vb) {
    gsub(/^[ \t]+|[ \t]+$/, "", el)
    if (el == "") return
    if (el ~ /^\{/) {
      if (!match(el, /run[ \t]*=[ \t]*"[^"]*"/)) return
      run = substr(el, RSTART, RLENGTH); sub(/run[ \t]*=[ \t]*"/, "", run); sub(/"$/, "", run)
      nm = run
      if (match(el, /name[ \t]*=[ \t]*"[^"]*"/)) {
        nm = substr(el, RSTART, RLENGTH); sub(/name[ \t]*=[ \t]*"/, "", nm); sub(/"$/, "", nm)
      }
      vb = (el ~ /verbose[ \t]*=[ \t]*true/) ? 1 : 0
      if (match(el, /on[ \t]*=[ \t]*"[^"]*"/)) {
        on = substr(el, RSTART, RLENGTH); sub(/on[ \t]*=[ \t]*"/, "", on); sub(/"$/, "", on)
        if (on != stage) return
      } else if (match(el, /on[ \t]*=[ \t]*\[[^]]*\]/)) {
        on = substr(el, RSTART, RLENGTH); if (on !~ ("\"" stage "\"")) return
      } else if (stage != "pre-commit") return
      printf "%d\034%s\034%s\n", vb, nm, run
    } else if (el ~ /^"/) {
      if (stage != "pre-commit") return
      run = el; sub(/^"/, "", run); sub(/"$/, "", run)
      printf "0\034%s\034%s\n", run, run
    }
  }
' "$config")

[ -n "$records" ] || exit 0

# Color: NO_COLOR disables; FORCE_COLOR/CLICOLOR_FORCE forces; else only on a TTY.
if   [ -n "${NO_COLOR:-}" ];                                   then G= ; R= ; Y= ; D= ; O=
elif [ -n "${FORCE_COLOR:-}${CLICOLOR_FORCE:-}" ] || [ -t 1 ]; then
     G='\033[32m'; R='\033[31m'; Y='\033[33m'; D='\033[2m'; O='\033[0m'
else G= ; R= ; Y= ; D= ; O= ; fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# Collect records into numbered files; pick up config flags.
printf '%s\n' "$records" | { while IFS= read -r line; do
  case $line in
    FAILFAST) echo 1 > "$tmp/ff";  continue ;;
    PARALLEL) echo 1 > "$tmp/par"; continue ;;
  esac
  i=$(($(cat "$tmp/n" 2>/dev/null || echo 0) + 1)); echo "$i" > "$tmp/n"
  printf '%s' "$line" > "$tmp/$i.rec"
done; }
fail_fast=0; [ -f "$tmp/ff" ]  && fail_fast=1
parallel=0;  [ -f "$tmp/par" ] && parallel=1
n=$(cat "$tmp/n" 2>/dev/null || echo 0)
[ "$n" -gt 0 ] || exit 0

# SKIP="a,b" skips those hooks by name.
is_skipped() { case ",${SKIP:-}," in *",$1,"*) return 0 ;; *) return 1 ;; esac }

run_one() {  # $1 index, rest forwarded as PREHOOK_ARGS; writes .out and .rc
  _i=$1; shift
  IFS=$SEP read -r _v _nm _cmd < "$tmp/$_i.rec"
  PREHOOK_ARGS="$*" sh -c "$_cmd" > "$tmp/$_i.out" 2>&1
  echo $? > "$tmp/$_i.rc"
}

# Parallel: launch all non-skipped at once, then wait.
if [ "$parallel" = 1 ] && [ "$n" -gt 1 ]; then
  i=1; while [ "$i" -le "$n" ]; do
    IFS=$SEP read -r v nm cmd < "$tmp/$i.rec"
    is_skipped "$nm" || { run_one "$i" "$@" & echo $! > "$tmp/$i.pid"; }
    i=$((i + 1))
  done
  i=1; while [ "$i" -le "$n" ]; do
    [ -f "$tmp/$i.pid" ] && wait "$(cat "$tmp/$i.pid")" 2>/dev/null
    i=$((i + 1))
  done
fi

# Report in order (and run, when sequential).
passed=0; failed=0; skipped=0; stop=0; i=1
while [ "$i" -le "$n" ]; do
  IFS=$SEP read -r v nm cmd < "$tmp/$i.rec"

  if is_skipped "$nm"; then
    printf '%b↷ %s (skipped)%b\n' "$D" "$nm" "$O"; skipped=$((skipped + 1)); i=$((i + 1)); continue
  fi
  [ "$stop" = 1 ] && break
  if [ "$parallel" != 1 ] || [ "$n" -le 1 ]; then run_one "$i" "$@"; fi

  rc=$(cat "$tmp/$i.rc" 2>/dev/null || echo 1)
  if [ "$rc" = 0 ]; then
    printf '%b✓%b %s\n' "$G" "$O" "$nm"; passed=$((passed + 1))
    [ "$v" = 1 ] && [ -s "$tmp/$i.out" ] && sed 's/^/  /' "$tmp/$i.out"
  else
    printf '%b✗%b %s\n' "$R" "$O" "$nm"; failed=$((failed + 1))
    [ -s "$tmp/$i.out" ] && sed 's/^/  /' "$tmp/$i.out"
    [ "$fail_fast" = 1 ] && stop=1
  fi
  i=$((i + 1))
done

if [ "$n" -gt 1 ]; then
  parts=
  [ "$passed"  -gt 0 ] && parts="${G}${passed} passed${O}"
  [ "$failed"  -gt 0 ] && parts="${parts:+$parts, }${R}${failed} failed${O}"
  [ "$skipped" -gt 0 ] && parts="${parts:+$parts, }${Y}${skipped} skipped${O}"
  printf '\n%b\n' "$parts"
fi

[ "$failed" -gt 0 ] && exit 1
exit 0
