package main

import (
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// Holding Cmd (Ctrl on Windows and Linux) alone for a moment puts each of the first chats' shortcuts
// in its stamp, and letting go takes them away.
func TestChatShortcutHints(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	tt.Frame()
	tt.HoldModifiers(ui.Cmd)
	if tt.HasText(shortcutText("CmdOrCtrl+1")) {
		t.Fatal("hints at once")
	}
	time.Sleep(300 * time.Millisecond)
	tt.Frame()
	renderTo(t, tt, "sidebar-shortcut-hints")
	if !tt.HasText(shortcutText("CmdOrCtrl+1")) || !tt.HasText(shortcutText("CmdOrCtrl+7")) {
		t.Fatalf("no hints: %q", tt.Texts())
	}
	tt.HoldModifiers(0)
	if tt.HasText(shortcutText("CmdOrCtrl+1")) {
		t.Fatal("hints stayed")
	}
}
