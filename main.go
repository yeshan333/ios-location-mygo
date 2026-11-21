package main

import (
	"embed"
	"io/fs"
	"log"

	"github.com/egoist/mygo"
	"ios-location-mygo/locales"
)

//go:embed all:web
var webAssets embed.FS

func main() {
	webFS, err := fs.Sub(webAssets, "web")
	if err != nil {
		log.Fatal(err)
	}
	mygo.SetFrontend(webFS)

	mygo.App.SetName("iOS Location")
	mygo.App.SetVersion("0.1.0")

	svc := NewSpooferService()
	mygo.BindAs("Spoofer", svc)

	openMainWindow := func() *mygo.Window {
		return mygo.NewWindow(mygo.WindowOptions{
			Title:                locales.AppTitle(),
			URL:                  "/",
			Width:                1180,
			Height:               780,
			MinWidth:             820,
			MinHeight:            560,
			TitleBarStyle:        mygo.TitleBarHiddenInset,
			TrafficLightPosition: &mygo.Point{X: 26, Y: 24},
			StateKey:             "main",
			BackgroundColor:      "light-dark(#f2f2f7, #161618)",
		})
	}

	mygo.App.WhenReady(func() {
		svc.InitDataDir()
		mygo.App.OnLocaleChanged(func(tag string) {
			_ = LocaleEvent.Broadcast(normalizeLocale(tag))
		})
		openMainWindow()
	})

	mygo.App.OnActivate(func(hasVisibleWindows bool) {
		if !hasVisibleWindows {
			openMainWindow()
		}
	})

	mygo.App.OnBeforeQuit(func(e *mygo.QuitEvent) {
		svc.Shutdown()
	})

	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
