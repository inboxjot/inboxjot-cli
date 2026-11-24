Output demos/out/agent.gif
Output demos/out/agent.mp4

Source demos/theme.tape

Hide
Type 'source "$INBOXJOT_DEMO_RC" && export INBOXJOT_HOME="$INBOXJOT_DEMO_AGENT_HOME" PATH="$INBOXJOT_DEMO_AGENT_PATH" && clear' Enter
Sleep 1s
Show

Sleep 500ms
Type "inboxjot agent setup"
Sleep 400ms
Enter
Wait+Screen /Which tools/
Sleep 2.5s
Enter
Wait+Screen /Configure Claude Code\?/
Sleep 1.5s
Type "y"
Wait+Screen /Configured Claude Code/
Sleep 5s
