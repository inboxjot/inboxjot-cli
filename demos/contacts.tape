Output demos/out/contacts.gif
Output demos/out/contacts.mp4

Source demos/theme.tape

Hide
Type 'source "$INBOXJOT_DEMO_RC" && clear' Enter
Sleep 1s
Show

Sleep 500ms
Type "cat customers.csv"
Sleep 400ms
Enter
Sleep 2.5s

Type "inboxjot contacts import customers.csv --dry-run"
Sleep 400ms
Enter
Sleep 3.5s

Hide
Type "clear" Enter
Sleep 500ms
Show

Type "inboxjot contacts import customers.csv"
Sleep 400ms
Enter
Wait+Screen /Imported/
Sleep 3s

Hide
Type "clear" Enter
Sleep 500ms
Show

Type "inboxjot contacts list --limit 5"
Sleep 400ms
Enter
Sleep 4s

Type `inboxjot contacts list -n 5 --jq '.data[] | select(.properties.plan == "pro").email'`
Sleep 400ms
Enter
Sleep 5s
