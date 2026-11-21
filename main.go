package main

import (
	"log"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"ios-location-mygo/locales"
)

func main() {
	mygo.App.SetName("iOS Location")
	mygo.App.SetVersion("0.2.0")

	svc := NewSpooferService()
	app := newNativeApp(svc)

	openMainWindow := func() *mygo.Window {
		w := mygo.NewWindow(mygo.WindowOptions{
			Title:                locales.AppTitle(),
			Content:              ui.View(app.view),
			Width:                1180,
			Height:               780,
			MinWidth:             820,
			MinHeight:            560,
			TitleBarStyle:        mygo.TitleBarHiddenInset,
			TrafficLightPosition: &mygo.Point{X: 26, Y: 24},
			StateKey:             "main",
			BackgroundColor:      "light-dark(#e8eef5, #161618)",
		})
		app.attachWindow(w)
		return w
	}

	mygo.App.WhenReady(func() {
		svc.InitDataDir()
		mygo.App.OnLocaleChanged(func(tag string) {
			app.invalidate()
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
