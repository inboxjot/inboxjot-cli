Output demos/out/send.gif
Output demos/out/send.mp4

Source demos/theme.tape

Hide
Type 'source "$INBOXJOT_DEMO_RC" && clear' Enter
Sleep 1s
Show

Sleep 500ms
Type `inboxjot send \`
Enter
Type `--from hi@acme.dev \`
Enter
Type `--to delivered@inboxjot.dev \`
Enter
Type `--subject "Welcome" \`
Enter
Type `--text "Hi Riti" \`
Enter
Type `--wait`
Sleep 400ms
Enter
Wait+Screen /Delivered to/
Sleep 3s

Hide
Type "clear" Enter
Sleep 300ms
Show

Type `inboxjot send \`
Enter
Type `--from hi@acme.dev \`
Enter
Type `--to bounced@inboxjot.dev \`
Enter
Type `--subject "Your receipt" \`
Enter
Type `--text "Thanks!" \`
Enter
Type `--wait`
Sleep 400ms
Enter
Wait+Screen /Couldn't deliver/
Sleep 5s
