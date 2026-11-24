Output demos/out/webhooks.gif
Output demos/out/webhooks.mp4

Source demos/theme.tape

Hide
Type 'source "$INBOXJOT_DEMO_RC" && clear' Enter
Sleep 1s
Type `(sleep 6; inboxjot send --from hi@acme.dev --to delivered@inboxjot.dev --subject "Your order shipped" --text "On its way" >/dev/null 2>&1; sleep 2; inboxjot trigger email.bounced >/dev/null 2>&1) & clear` Enter
Sleep 500ms
Show

Sleep 500ms
Type "inboxjot listen --forward-to localhost:4242/webhooks"
Sleep 400ms
Enter
Wait+Screen /email\.bounced .*200/
Sleep 4s
Ctrl+C
Sleep 1.5s
