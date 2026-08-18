#!/usr/bin/env bash
#
# This script is for runtime differential testing. For every Limbo program that BOTH the reference
# compiler (limbo.dis, run on RiceVM) and the built-in compiler (ricevm-limbo)
# accept, run both binaries on RiceVM and compare what they actually print.
#
# Compiling proves very little on its own -- a miscompiled program compiles
# fine and prints a plausible wrong answer. This compares behaviour instead.
#
#   scripts/diff-runtime.sh [N]     # N = only look at the first N sources
#
# Output: a per-program verdict, then a summary. Details land in
# $OUTDIR/report.tsv, and every mismatch keeps both outputs for inspection.
#
# Exit 0 if no program that both compilers accepted disagreed at run time.
set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

RICEVM="$ROOT/target/release/ricevm-cli"
LIMBO_DIS="$ROOT/external/inferno-os/dis/limbo.dis"
MODULES="$ROOT/external/inferno-os/module"
APPL="$ROOT/external/inferno-os/appl"
PROBE=(--probe "$ROOT/external/inferno-os/dis" --probe "$ROOT/external/inferno-os/dis/lib")
OUTDIR=${DIFF_RUNTIME_OUT:-"$ROOT/target/diff-runtime"}
LIMIT=${1:-0}

[ -x "$RICEVM" ] || { echo "build first: cargo build --release -p ricevm-cli"; exit 2; }
[ -f "$LIMBO_DIS" ] || { echo "reference compiler not found: $LIMBO_DIS"; exit 2; }

rm -rf "$OUTDIR"; mkdir -p "$OUTDIR/mismatch"
printf 'verdict\tprogram\tnote\n' > "$OUTDIR/report.tsv"

same=0 diff=0 ours_only=0 ref_only=0 neither=0 n=0

# Strip the CLI's own chatter so only guest output is compared.
strip_chatter() { grep -v '^✓\|INFO\|Module loaded\|^$' || true; }

# The reference compiler writes <ModuleName>.dis into the current directory,
# and the module name need not match the file name -- so compile inside an
# empty directory and take whatever .dis appears.
compile_ref() { # $1 = source, $2 = destination .dis
	local src=$1 dst=$2 work
	work=$(mktemp -d)
	# stdin must be closed off: this runs inside a `while read` loop, and a
	# child inheriting the loop's stdin would swallow the program list.
	( cd "$work" && timeout 60 "$RICEVM" run "$LIMBO_DIS" "${PROBE[@]}" \
		-- -I "$MODULES" "$src" ) </dev/null >/dev/null 2>&1
	local produced
	produced=$(find "$work" -maxdepth 1 -name '*.dis' -print -quit 2>/dev/null)
	if [ -n "$produced" ]; then mv "$produced" "$dst"; rm -rf "$work"; return 0; fi
	rm -rf "$work"; return 1
}

run_guest() { # $1 = .dis -> stdout of the guest, chatter removed
	timeout 20 "$RICEVM" run "$1" "${PROBE[@]}" </dev/null 2>/dev/null | strip_chatter
}

while IFS= read -r src; do
	n=$((n + 1))
	[ "$LIMIT" -gt 0 ] && [ "$n" -gt "$LIMIT" ] && break
	name=$(basename "$src" .b)
	rel=${src#"$ROOT/"}

	ref_dis="$OUTDIR/${name}.ref.dis"
	our_dis="$OUTDIR/${name}.our.dis"

	compile_ref "$src" "$ref_dis" && has_ref=1 || has_ref=0
	if timeout 20 "$RICEVM" compile "$src" -I "$MODULES" -o "$our_dis" \
		</dev/null >/dev/null 2>&1
	then has_our=1; else has_our=0; fi

	if [ "$has_ref" = 0 ] && [ "$has_our" = 0 ]; then
		neither=$((neither + 1)); printf 'NEITHER\t%s\t\n' "$rel" >> "$OUTDIR/report.tsv"
		rm -f "$ref_dis" "$our_dis"; continue
	fi
	if [ "$has_ref" = 1 ] && [ "$has_our" = 0 ]; then
		ref_only=$((ref_only + 1)); printf 'REF_ONLY\t%s\t\n' "$rel" >> "$OUTDIR/report.tsv"
		rm -f "$ref_dis" "$our_dis"; continue
	fi
	if [ "$has_ref" = 0 ] && [ "$has_our" = 1 ]; then
		# We accepted something the reference rejected: worth a look either way.
		ours_only=$((ours_only + 1)); printf 'OURS_ONLY\t%s\t\n' "$rel" >> "$OUTDIR/report.tsv"
		rm -f "$ref_dis" "$our_dis"; continue
	fi

	# Both compiled: compare behaviour. No arguments and empty stdin, so a
	# program that needs them prints its usage message -- itself a fine
	# deterministic signal to compare.
	ref_out=$(run_guest "$ref_dis")
	our_out=$(run_guest "$our_dis")

	if [ "$ref_out" = "$our_out" ]; then
		same=$((same + 1)); printf 'SAME\t%s\t\n' "$rel" >> "$OUTDIR/report.tsv"
		rm -f "$ref_dis" "$our_dis"
	else
		diff=$((diff + 1))
		printf '%s\n' "$ref_out" > "$OUTDIR/mismatch/${name}.ref.txt"
		printf '%s\n' "$our_out" > "$OUTDIR/mismatch/${name}.our.txt"
		printf 'DIFF\t%s\tref=%q ours=%q\n' "$rel" \
			"$(printf '%s' "$ref_out" | head -c 120)" \
			"$(printf '%s' "$our_out" | head -c 120)" >> "$OUTDIR/report.tsv"
		echo "DIFF $rel"
	fi
done < <(find "$APPL" -name '*.b' | sort)

cat <<EOF

=== runtime differential ===
both compiled, same output : $same
both compiled, DIFFERENT   : $diff
reference only compiled    : $ref_only
built-in only compiled     : $ours_only
neither compiled           : $neither
report: $OUTDIR/report.tsv
EOF

[ "$diff" -eq 0 ]
