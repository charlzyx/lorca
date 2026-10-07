package main

import (
	"testing"

	"github.com/egoist/mygo/ui"
)

// A drag runs across a message's paragraphs, list, and code, and Copy joins them a line apart,
// without the list's markers.
func TestMessageSelectsAcrossBlocks(t *testing.T) {
	text := "First paragraph.\n\nSecond paragraph.\n\n- one\n- two\n\n```\ncode line\n```"
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		ui.Column(c).Padding(20).Width(360).Children(func() { markdownView(c, text, markdownOptions{}) })
	}, 400, 400)
	first, ok := tt.Find("First paragraph.")
	if !ok {
		t.Fatalf("no first paragraph: %q", tt.Texts())
	}
	last, ok := tt.Find("code line")
	if !ok {
		t.Fatalf("no code: %q", tt.Texts())
	}
	tt.Press(first.X+1, first.Y+first.H/2)
	tt.Move(last.X+last.W/2, last.Y+last.H/2)
	tt.Release(last.X+last.W-1, last.Y+last.H/2)
	tt.Key(ui.Cmd, ui.KeyC)
	want := "First paragraph.\nSecond paragraph.\none\ntwo\ncode line"
	if got := tt.Clipboard(); got != want {
		t.Errorf("copied %q, want %q", got, want)
	}
}
