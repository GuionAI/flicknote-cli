Content is read from stdin and inserted next to the selected section.
Section IDs come from `flicknote detail <id> --tree`.

Example:
cat <<'EOF' | flicknote insert 123 --after 3K
## New section

Add the new section body here.
EOF
