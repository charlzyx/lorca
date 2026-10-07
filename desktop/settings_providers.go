package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// providers is the account's provider credentials: connected on any Device, used by every Runner.
// The built-in providers come first, then the custom ones in the order they were added, and a row
// to add one.
func (s settingsPane) providers() {
	c := s.c
	p := colors(c)
	s.frame(string(model.PaneProviders), func() {
		s.section(L("Credentials"), nil, func(k *card) {
			if len(store.Providers) == 0 {
				keyValueRow(k, L("Waiting for the CLI"), "", false, nil)
				return
			}
			for _, credential := range store.Providers {
				kind := credential.Kind
				// A subscription disconnects right here; an API key or a custom provider opens its sheet.
				disconnects := credential.IsConnected && !model.UsesAPIKey(kind) && !model.IsCustomKind(kind)
				action := L("Connect…")
				if credential.IsConnected {
					action = L("Edit…")
					if disconnects {
						action = L("Disconnect")
					}
				}
				o := statusRowOptions{
					Symbol:      model.ProviderSymbol(kind),
					Title:       model.ProviderName(kind, store.Providers),
					Subtitle:    model.ProviderSubtitle(kind) + " · " + credential.Detail,
					StateColor:  &p.Green,
					ActionTitle: action,
					Destructive: disconnects,
				}
				if credential.IsConnected {
					o.State = L("Connected")
				}
				ui.Column(c).Key(kind).Children(func() {
					row := statusRow(k, o)
					s.mark(row.Row, o.Title)
					if !row.Action {
						return
					}
					switch {
					case model.IsCustomKind(kind):
						s.w.presentCustomProvider(kind, nil, nil)
					case disconnects:
						store.DisconnectProvider(kind, func(error) {})
					default:
						s.w.presentConnectProvider(kind, credential.BaseURL, nil)
					}
				})
			}
			add := actionRow(k, L("Custom"), actionRowOptions{Tint: &p.Label2, Action: L("Add Provider…")})
			s.mark(add.Row, L("Custom"))
			add.Action.Menu(s.w.addProviderMenu)
		})
		s.footnote(L("Credentials belong to your account. They reach your paired Devices encrypted with the account key, so a bot uses them on whichever Runner it is assigned to; the relay stores ciphertext."))
	})
}
