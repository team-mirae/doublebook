#!/bin/sh
# Writes one JSON array per accountant table into DIR, for
# `doublebook migrate-terraform DIR`.
#
#   scripts/export-terraform.sh postgres "$DATABASE_URL" DIR   # Terraform (acct_* tables)
#   scripts/export-terraform.sh sqlite accountant.db DIR       # magic-company backup
set -eu
kind=$1 src=$2 dir=$3
mkdir -p "$dir"
for t in books currencies accounts clients batches entries postings entry_revisions cash_flow_pairs statement_notes; do
  case $kind in
    postgres)
      psql "$src" -X -A -t -v ON_ERROR_STOP=1 \
        -c "SELECT coalesce(json_agg(t), '[]') FROM acct_$t t" > "$dir/$t.json" ;;
    sqlite)
      out=$(sqlite3 -json "$src" "SELECT * FROM $t")
      printf '%s\n' "${out:-[]}" > "$dir/$t.json" ;;
    *) echo "unknown source kind: $kind" >&2; exit 2 ;;
  esac
done
