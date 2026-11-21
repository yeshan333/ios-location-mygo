package main

import (
	"context"
	_ "embed"
	"fmt"
	"math"
	"strings"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/glass"
	"github.com/egoist/mygo/ui"
	"ios-location-mygo/locales"
)

//go:embed assets/app-icon.png
var appIconPNG []byte

func svgIcon(inner string) *ui.SVG {
	return ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">` + inner + `</svg>`))
}

var (
	icSun       = svgIcon(`<circle cx="12" cy="12" r="4"/><path d="M12 2v2.2M12 19.8V22M4.93 4.93l1.56 1.56M17.51 17.51l1.56 1.56M2 12h2.2M19.8 12H22M4.93 19.07l1.56-1.56M17.51 6.49l1.56-1.56"/>`)
	icMoon      = svgIcon(`<path d="M20.5 14.5A8.5 8.5 0 0 1 9.5 3.5a8.5 8.5 0 1 0 11 11z"/>`)
	icSidebar   = svgIcon(`<rect x="3" y="4" width="18" height="16" rx="3"/><line x1="9" y1="4" x2="9" y2="20"/>`)
	icWifi      = svgIcon(`<path d="M5 12.55a11 11 0 0 1 14.08 0"/><path d="M8.53 16.11a6 6 0 0 1 6.95 0"/><circle cx="12" cy="19.5" r="1" fill="currentColor" stroke="none"/>`)
	icUSB       = svgIcon(`<path d="M12 2v14"/><path d="M9 5l3-3 3 3"/><path d="M12 12l5-3v-2"/><path d="M12 14l-5-3V9"/><circle cx="12" cy="19" r="2.5"/>`)
	icOffline   = svgIcon(`<circle cx="12" cy="12" r="8"/><line x1="6.3" y1="6.3" x2="17.7" y2="17.7"/>`)
	icPhone     = svgIcon(`<rect x="6" y="2.5" width="12" height="19" rx="2.5"/><line x1="10.5" y1="18" x2="13.5" y2="18"/>`)
	icRefresh   = svgIcon(`<path d="M21 12a9 9 0 1 1-2.64-6.36"/><polyline points="21 3 21 9 15 9"/>`)
	icCopy      = svgIcon(`<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>`)
	icCheck     = svgIcon(`<polyline points="20 6 9 17 4 12"/>`)
	icTarget    = svgIcon(`<circle cx="12" cy="12" r="7"/><line x1="12" y1="2" x2="12" y2="5"/><line x1="12" y1="19" x2="12" y2="22"/><line x1="2" y1="12" x2="5" y2="12"/><line x1="19" y1="12" x2="22" y2="12"/><circle cx="12" cy="12" r="2" fill="currentColor" stroke="none"/>`)
	icReset     = svgIcon(`<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><polyline points="3 3 3 8 8 8"/>`)
	icPlus      = svgIcon(`<line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>`)
	icMinus     = svgIcon(`<line x1="5" y1="12" x2="19" y2="12"/>`)
	icTrash     = svgIcon(`<polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/>`)
	icTeleport  = svgIcon(`<polygon points="3 11 22 2 13 21 11 13 3 11"/>`)
	icTerminal  = svgIcon(`<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>`)
	icChevronUp = svgIcon(`<polyline points="18 15 12 9 6 15"/>`)
	icChevronDn = svgIcon(`<polyline points="6 9 12 15 18 9"/>`)
	icChevUD    = svgIcon(`<path d="M8 9l4-4 4 4"/><path d="M8 15l4 4 4-4"/>`)
	icClose     = svgIcon(`<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>`)
	icPin       = svgIcon(`<path d="M12 21s-6-5.2-6-10.5a6 6 0 1 1 12 0C18 15.8 12 21 12 21z"/><circle cx="12" cy="10.5" r="2"/>`)
)

type presetCity struct {
	name   func() string
	region string
	lat    float64
	lng    float64
}

var presetCities = []presetCity{
	{locales.CityLA, "US", 34.0536923, -118.2427676},
	{locales.CitySF, "US", 37.7749295, -122.4194155},
	{locales.CityTokyo, "JP", 35.6585805, 139.7454329},
	{locales.CitySG, "SG", 1.2833754, 103.8607264},
	{locales.CityLondon, "UK", 51.5007292, -0.1246254},
	{locales.CityBeijing, "CN", 39.90872, 116.39748},
}

type nativeApp struct {
	svc     *SpooferService
	win     *mygo.Window
	tiles   *TileManager
	appIcon *ui.Bitmap

	// Map state (always stored in WGS-84)
	wgsLat    float64
	wgsLng    float64
	zoom      float64
	layer     MapLayer
	mapRect   ui.Rect
	dragging  bool
	dragMoved bool
	dragStart ui.Point
	dragWorld [2]float64

	// Device & spoofer state mirrors (updated on main thread via win.Update)
	devices        []Device
	devicesScanned bool
	scanning       bool
	selectedUDID   string
	state          SpooferState
	pairing        PairingState
	pairOpen       bool
	prevPaired     bool

	// UI state
	collapsed     bool
	tab           int // 0 = cities, 1 = favorites
	logOpen       bool
	favFormOpen   bool
	favName       string
	favorites     []Favorite
	searchQuery   string
	searchHandle  ui.Handle
	searchOpen    bool
	searchBusy    bool
	searchResults []SearchResult
	searchErr     string
	copiedUntil   time.Time
	pendingToast  string
}

func newNativeApp(svc *SpooferService) *nativeApp {
	iconBM, _ := ui.DecodeBitmap(appIconPNG)
	a := &nativeApp{
		svc:     svc,
		appIcon: iconBM,
		wgsLat:  34.0536923,
		wgsLng:  -118.2427676,
		zoom:    13.0,
		layer:   LayerGoogleMaps,
		state:   svc.GetState(),
		pairing: svc.GetPairingState(),
	}
	a.tiles = NewTileManager(func() {
		a.invalidate()
	})
	svc.SetOnChange(func() {
		a.syncBackendState()
	})
	return a
}

func (a *nativeApp) attachWindow(win *mygo.Window) {
	a.win = win
	a.reloadFavorites()
	a.refreshDevices()
}

func (a *nativeApp) invalidate() {
	if w := a.win; w != nil {
		w.Update(func() {})
	}
}

func (a *nativeApp) syncBackendState() {
	st := a.svc.GetState()
	ps := a.svc.GetPairingState()
	if w := a.win; w != nil {
		w.Update(func() {
			a.state = st
			a.pairing = ps
			if ps.Status == "paired" && !a.prevPaired {
				a.prevPaired = true
				name := ps.DeviceName
				if name == "" {
					name = "iPhone"
				}
				a.pendingToast = locales.ToastPaired(name)
				a.pairOpen = false
				a.refreshDevices()
			} else if ps.Status != "paired" {
				a.prevPaired = false
			}
		})
	}
}

func (a *nativeApp) reloadFavorites() {
	favs, err := a.svc.ListFavorites()
	if err != nil {
		return
	}
	if w := a.win; w != nil {
		w.Update(func() { a.favorites = favs })
	} else {
		a.favorites = favs
	}
}

func (a *nativeApp) refreshDevices() {
	if a.scanning {
		return
	}
	a.scanning = true
	go func() {
		devs, _ := a.svc.ListDevices(context.Background())
		if w := a.win; w != nil {
			w.Update(func() {
				a.scanning = false
				a.devicesScanned = true
				a.devices = devs
				found := false
				for _, d := range devs {
					if d.UDID == a.selectedUDID {
						found = true
						break
					}
				}
				if !found {
					if len(devs) > 0 {
						a.selectedUDID = devs[0].UDID
					} else {
						a.selectedUDID = ""
					}
				}
			})
		}
	}()
}

func (a *nativeApp) googleLang() string {
	if a.svc.GetLocale() == "zh-Hans" {
		return "zh-CN"
	}
	return "en"
}

func (a *nativeApp) wgsToDisp(lat, lng float64) (float64, float64) {
	if a.layer.isGCJ() {
		return wgsToGCJ(lat, lng)
	}
	return lat, lng
}

func (a *nativeApp) dispToWgs(lat, lng float64) (float64, float64) {
	if a.layer.isGCJ() {
		return gcjToWGS(lat, lng)
	}
	return lat, lng
}

func (a *nativeApp) jumpToWGS(lat, lng, minZoom float64) {
	a.wgsLat = clampMercatorLat(lat)
	a.wgsLng = lng
	if a.zoom < minZoom {
		a.zoom = minZoom
	}
	if maxZ := float64(a.layer.maxZoom()); a.zoom > maxZ {
		a.zoom = maxZ
	}
}

func (a *nativeApp) zoomAround(mx, my float64, deltaZoom float64) {
	maxZ := float64(a.layer.maxZoom())
	oldZoom := a.zoom
	newZoom := oldZoom + deltaZoom
	if newZoom < 2.5 {
		newZoom = 2.5
	}
	if newZoom > maxZ {
		newZoom = maxZ
	}
	if math.Abs(newZoom-oldZoom) < 1e-4 {
		return
	}
	cx := float64(a.mapRect.W) * 0.5
	cy := float64(a.mapRect.H) * 0.5
	offX := mx - cx
	offY := my - cy

	dlat, dlng := a.wgsToDisp(a.wgsLat, a.wgsLng)
	oldWX, oldWY := latLngToWorldPx(dlat, dlng, oldZoom)
	cLat, cLng := worldPxToLatLng(oldWX+offX, oldWY+offY, oldZoom)
	newCWX, newCWY := latLngToWorldPx(cLat, cLng, newZoom)
	nLat, nLng := worldPxToLatLng(newCWX-offX, newCWY-offY, newZoom)
	a.zoom = newZoom
	a.wgsLat, a.wgsLng = a.dispToWgs(nLat, nLng)
}

func (a *nativeApp) lockLocation(c *ui.Context, lat, lng float64, label string) {
	if a.selectedUDID == "" {
		c.Toast(locales.ToastNeedDevice())
		return
	}
	udid := a.selectedUDID
	go func() {
		_, err := a.svc.SetLocation(context.Background(), SetLocationRequest{
			UDID:      udid,
			Latitude:  lat,
			Longitude: lng,
			Label:     label,
		})
		if w := a.win; w != nil {
			w.Update(func() {
				if err != nil {
					a.pendingToast = locales.ToastSetFailed()
				} else {
					a.pendingToast = locales.ToastLocked()
				}
			})
		}
	}()
}

func (a *nativeApp) clearLocation() {
	udid := a.selectedUDID
	go func() {
		_, err := a.svc.ClearLocation(context.Background(), udid)
		if w := a.win; w != nil {
			w.Update(func() {
				if err != nil {
					a.pendingToast = locales.ToastClearFailed()
				} else {
					a.pendingToast = locales.ToastCleared()
				}
			})
		}
	}()
}

func (a *nativeApp) runSearch(query string) {
	q := strings.TrimSpace(query)
	if q == "" {
		return
	}
	a.searchOpen = true
	a.searchBusy = true
	a.searchErr = ""
	a.searchResults = nil
	go func() {
		items, err := a.svc.SearchPlace(context.Background(), q)
		if w := a.win; w != nil {
			w.Update(func() {
				a.searchBusy = false
				if err != nil {
					a.searchErr = locales.SearchFailed(err.Error())
					return
				}
				if len(items) == 1 && items[0].Direct {
					a.jumpToWGS(items[0].Latitude, items[0].Longitude, 15)
					a.searchOpen = false
					a.pendingToast = locales.JumpedCoord()
					return
				}
				a.searchResults = items
			})
		}
	}()
}

func (a *nativeApp) saveFavorite(c *ui.Context) {
	name := strings.TrimSpace(a.favName)
	if name == "" {
		name = fmt.Sprintf("%.4f, %.4f", a.wgsLat, a.wgsLng)
	}
	if _, err := a.svc.AddFavorite(name, a.wgsLat, a.wgsLng); err != nil {
		c.Toast(err.Error())
		return
	}
	a.favName = ""
	a.favFormOpen = false
	a.tab = 1
	a.reloadFavorites()
	c.Toast(locales.FavAdded())
}

func (a *nativeApp) copyCoords(c *ui.Context) {
	txt := fmt.Sprintf("%.6f, %.6f", a.wgsLat, a.wgsLng)
	c.WriteClipboard(txt)
	a.copiedUntil = c.Now().Add(1500 * time.Millisecond)
	c.Toast(locales.CopiedToast(txt))
}

func (a *nativeApp) openPairing() {
	a.pairOpen = true
	go func() {
		_, _ = a.svc.StartWifiPairing("MacBook-Location")
	}()
}

func (a *nativeApp) closePairing() {
	a.pairOpen = false
	go func() {
		a.svc.StopWifiPairing()
	}()
}

// --- Colors derived from current Light / Dark theme ---

type palette struct {
	dark         bool
	canvasBg     ui.Color
	glassTint    ui.Color
	glassPill    ui.Color
	panelBorder  ui.Color
	cardBg       ui.Color
	inputBg      ui.Color
	surface      ui.Color
	surfaceHover ui.Color
	divider      ui.Color
	text         ui.Color
	textSub      ui.Color
	muted        ui.Color
	accent       ui.Color
	accentSoft   ui.Color
	accentBorder ui.Color
	green        ui.Color
	greenSoft    ui.Color
	red          ui.Color
	redSoft      ui.Color
	amber        ui.Color
	logBg        ui.Color
}

func makePalette(t *ui.Theme) palette {
	if t.Dark {
		return palette{
			dark:         true,
			canvasBg:     ui.RGB(22, 22, 24),
			glassTint:    ui.RGBA(20, 20, 24, 0.56),
			glassPill:    ui.RGBA(20, 20, 24, 0.52),
			panelBorder:  ui.RGBA(255, 255, 255, 0.16),
			cardBg:       ui.RGBA(255, 255, 255, 0.06),
			inputBg:      ui.RGBA(0, 0, 0, 0.34),
			surface:      ui.RGBA(255, 255, 255, 0.09),
			surfaceHover: ui.RGBA(255, 255, 255, 0.16),
			divider:      ui.RGBA(255, 255, 255, 0.12),
			text:         ui.RGB(248, 248, 250),
			textSub:      ui.RGB(228, 228, 234),
			muted:        ui.RGB(186, 188, 198),
			accent:       ui.RGB(10, 132, 255),
			accentSoft:   ui.RGBA(10, 132, 255, 0.24),
			accentBorder: ui.RGBA(10, 132, 255, 0.50),
			green:        ui.RGB(64, 224, 104),
			greenSoft:    ui.RGBA(48, 209, 88, 0.22),
			red:          ui.RGB(255, 79, 68),
			redSoft:      ui.RGBA(255, 69, 58, 0.20),
			amber:        ui.RGB(255, 169, 20),
			logBg:        ui.RGBA(12, 12, 14, 0.78),
		}
	}
	return palette{
		dark:         false,
		canvasBg:     ui.RGB(232, 238, 245),
		glassTint:    ui.RGBA(250, 250, 254, 0.48),
		glassPill:    ui.RGBA(250, 250, 254, 0.45),
		panelBorder:  ui.RGBA(0, 0, 0, 0.11),
		cardBg:       ui.RGBA(0, 0, 0, 0.04),
		inputBg:      ui.RGBA(0, 0, 0, 0.065),
		surface:      ui.RGBA(0, 0, 0, 0.055),
		surfaceHover: ui.RGBA(0, 0, 0, 0.10),
		divider:      ui.RGBA(0, 0, 0, 0.10),
		text:         ui.RGB(24, 24, 28),
		textSub:      ui.RGB(64, 64, 70),
		muted:        ui.RGB(102, 102, 112),
		accent:       ui.RGB(0, 122, 255),
		accentSoft:   ui.RGBA(0, 122, 255, 0.14),
		accentBorder: ui.RGBA(0, 122, 255, 0.38),
		green:        ui.RGB(28, 178, 62),
		greenSoft:    ui.RGBA(40, 205, 65, 0.16),
		red:          ui.RGB(235, 50, 40),
		redSoft:      ui.RGBA(255, 59, 48, 0.14),
		amber:        ui.RGB(235, 138, 0),
		logBg:        ui.RGBA(242, 242, 247, 0.88),
	}
}

func (a *nativeApp) view(c *ui.Context) {
	// Flush any toast queued from background goroutines.
	if a.pendingToast != "" {
		c.Toast(a.pendingToast)
		a.pendingToast = ""
	}

	// Global keyboard shortcuts.
	if c.Shortcut(ui.Cmd, ui.KeyBackslash) {
		a.collapsed = !a.collapsed
	}
	if c.Shortcut(ui.Cmd, ui.KeyK) {
		a.collapsed = false
		a.searchHandle.Focus(c)
	}

	pal := makePalette(c.Theme())

	ui.Box(c).Fill().Background(pal.canvasBg).Children(func() {
		// 1. Full-bleed native GPU Slippy Map canvas
		a.buildMapCanvas(c, pal)

		// 2. Floating Liquid Glass Inspector Panel (or Collapsed Pill)
		if a.collapsed {
			a.buildCollapsedPill(c, pal)
		} else {
			a.buildInspector(c, pal)
		}

		// 3. Top-right Liquid Glass Layer Switcher
		a.buildLayerBar(c, pal)

		// 4. Bottom-right Liquid Glass Map Controls
		a.buildMapControls(c, pal)

		// 5. Bottom-center Liquid Glass Coordinate HUD Pill
		a.buildBottomHUD(c, pal)

		// 6. Map Attribution Badge
		ui.Box(c).Absolute().Bottom(6).Right(12).Padding(2, 6).Radius(5).
			Background(ui.RGBA(0, 0, 0, 0.32)).PassThrough().Children(func() {
			ui.Text(c, a.layer.attribution()).FontSize(10).TextColor(ui.RGBA(255, 255, 255, 0.85))
		})

		// 7. iOS 27+ Wi-Fi RemotePairing Modal
		a.buildPairingDialog(c, pal)
	})
}

func (a *nativeApp) buildMapCanvas(c *ui.Context, pal palette) {
	cursor := ui.CursorGrab
	if a.dragging {
		cursor = ui.CursorGrabbing
	}

	lang := a.googleLang()
	ui.Box(c).Absolute().Fill().Cursor(cursor).
		Draw(func(p *ui.Painter, r ui.Rect) {
			a.mapRect = r
			w := float64(r.W)
			h := float64(r.H)
			if w <= 1 || h <= 1 {
				return
			}

			z := int(math.Floor(a.zoom))
			if z < 2 {
				z = 2
			}
			if z > a.layer.maxZoom() {
				z = a.layer.maxZoom()
			}
			scale := math.Pow(2.0, a.zoom-float64(z))
			tPx := tileSize * scale
			nTiles := 1 << z

			dLat, dLng := a.wgsToDisp(a.wgsLat, a.wgsLng)
			centerWX, centerWY := latLngToWorldPx(dLat, dLng, a.zoom)
			tlWX := centerWX - w*0.5
			tlWY := centerWY - h*0.5

			minTX := int(math.Floor(tlWX/tPx)) - 1
			maxTX := int(math.Ceil((tlWX + w) / tPx))
			minTY := max(int(math.Floor(tlWY/tPx)), 0)
			maxTY := min(int(math.Ceil((tlWY+h)/tPx)), nTiles-1)

			tileW := float32(tPx) + 0.6
			tileH := float32(tPx) + 0.6
			missing := 0

			for ty := minTY; ty <= maxTY; ty++ {
				for tx := minTX; tx <= maxTX; tx++ {
					wrappedTX := ((tx % nTiles) + nTiles) % nTiles
					sx := r.X + float32(float64(tx)*tPx-tlWX)
					sy := r.Y + float32(float64(ty)*tPx-tlWY)
					dst := ui.Rect{X: sx, Y: sy, W: tileW, H: tileH}

					key := tileKey{layer: a.layer, lang: lang, overlay: false, z: z, x: wrappedTX, y: ty}
					if bm := a.tiles.GetOrRequest(key); bm != nil {
						p.Image(bm, dst, ui.FillBox)
					} else {
						missing++
						p.Fill(dst, ui.RGBA(130, 148, 168, 0.08), 0)
					}

					if a.layer.hasOverlay() {
						ovKey := key
						ovKey.overlay = true
						if ovBM := a.tiles.GetOrRequest(ovKey); ovBM != nil {
							p.Image(ovBM, dst, ui.FillBox)
						} else {
							missing++
						}
					}
				}
			}
			if missing > 0 {
				p.After(60 * time.Millisecond)
			}

			// Locked location beacon
			if a.state.Active {
				ldLat, ldLng := a.wgsToDisp(a.state.Latitude, a.state.Longitude)
				lwx, lwy := latLngToWorldPx(ldLat, ldLng, a.zoom)
				lx := r.X + float32(lwx-tlWX)
				ly := r.Y + float32(lwy-tlWY)
				if lx >= r.X-30 && lx <= r.X+r.W+30 && ly >= r.Y-30 && ly <= r.Y+r.H+30 {
					halo := (&ui.Path{}).Circle(lx, ly, 18)
					p.FillPath(halo, ui.RGBA(0, 122, 255, 0.24))
					ring := (&ui.Path{}).Circle(lx, ly, 8.5)
					p.FillPath(ring, ui.RGB(255, 255, 255))
					core := (&ui.Path{}).Circle(lx, ly, 6.2)
					p.FillPath(core, ui.RGB(0, 122, 255))
				}
			}

			// Center precision reticle crosshair
			cx := r.X + r.W*0.5
			cy := r.Y + r.H*0.5
			rad := float32(10.0)
			if a.dragging && a.dragMoved {
				rad = 12.0
			}
			blue := ui.RGB(0, 122, 255)
			whiteHalo := ui.RGBA(255, 255, 255, 0.82)

			ring := (&ui.Path{}).Circle(cx, cy, rad)
			p.StrokePath(ring, 3.2, whiteHalo)
			p.StrokePath(ring, 1.8, blue)

			tick := float32(8.0)
			gap := rad + 1.0
			p.Line(cx, cy-gap-tick, cx, cy-gap, 2.0, blue)
			p.Line(cx, cy+gap, cx, cy+gap+tick, 2.0, blue)
			p.Line(cx-gap-tick, cy, cx-gap, cy, 2.0, blue)
			p.Line(cx+gap, cy, cx+gap+tick, cy, 2.0, blue)

			dotOuter := (&ui.Path{}).Circle(cx, cy, 3.2)
			p.FillPath(dotOuter, ui.RGB(255, 255, 255))
			dotInner := (&ui.Path{}).Circle(cx, cy, 2.1)
			p.FillPath(dotInner, blue)
		}).
		HandleInput(func(ev ui.InputEvent) bool {
			switch ev.Kind {
			case ui.InputPointerDown:
				if ev.Button != 0 {
					return false
				}
				a.searchOpen = false
				a.dragging = true
				a.dragMoved = false
				a.dragStart = ui.Point{X: ev.X, Y: ev.Y}
				dlat, dlng := a.wgsToDisp(a.wgsLat, a.wgsLng)
				wx, wy := latLngToWorldPx(dlat, dlng, a.zoom)
				a.dragWorld = [2]float64{wx, wy}
				return true

			case ui.InputPointerMove:
				if !a.dragging || ev.Button != 0 {
					return false
				}
				dx := float64(ev.X - a.dragStart.X)
				dy := float64(ev.Y - a.dragStart.Y)
				if math.Abs(dx) > 3 || math.Abs(dy) > 3 {
					a.dragMoved = true
				}
				nLat, nLng := worldPxToLatLng(a.dragWorld[0]-dx, a.dragWorld[1]-dy, a.zoom)
				a.wgsLat, a.wgsLng = a.dispToWgs(nLat, nLng)
				return true

			case ui.InputPointerUp:
				if !a.dragging {
					return false
				}
				a.dragging = false
				if !ev.Canceled && !a.dragMoved {
					cx := float64(a.mapRect.W) * 0.5
					cy := float64(a.mapRect.H) * 0.5
					dlat, dlng := a.wgsToDisp(a.wgsLat, a.wgsLng)
					wx, wy := latLngToWorldPx(dlat, dlng, a.zoom)
					nLat, nLng := worldPxToLatLng(wx+float64(ev.X)-cx, wy+float64(ev.Y)-cy, a.zoom)
					a.wgsLat, a.wgsLng = a.dispToWgs(nLat, nLng)
				}
				return true

			case ui.InputScroll:
				// Negative DY scrolls up -> zoom in
				var dz float64
				if ev.Precise {
					dz = -float64(ev.DY) / 200.0
				} else {
					dz = -float64(ev.DY) / 100.0
				}
				if math.Abs(dz) > 1e-4 {
					a.zoomAround(float64(ev.X), float64(ev.Y), dz)
				}
				return true

			case ui.InputZoom:
				if ev.Scale > 0 {
					a.zoomAround(float64(ev.X), float64(ev.Y), math.Log2(float64(ev.Scale)))
				}
				return true
			}
			return false
		})
}

func (a *nativeApp) buildCollapsedPill(c *ui.Context, pal palette) {
	btn := ui.ButtonBase(c).Row().Absolute().Top(14).Left(96).Height(32).Padding(0, 12).Gap(7).
		Radius(11).Material(glass.Glass{Tint: pal.glassPill, Interactive: true}).
		Border(1, pal.panelBorder).AlignItems(ui.Center).Cursor(ui.CursorPointer).
		Children(func() {
			ui.Icon(c, icSidebar).Size(14, 14).TextColor(pal.text)
			ui.Text(c, locales.ExpandLabel()).FontSize(12).FontWeight(600).TextColor(pal.text)
			if a.state.Active {
				ui.Box(c).Size(7, 7).Radius(99).Background(pal.green)
			}
		})
	if btn.Clicked() {
		a.collapsed = false
	}
}

func (a *nativeApp) buildInspector(c *ui.Context, pal palette) {
	// Floating Liquid Glass panel over the map.
	panel := ui.Column(c).Absolute().Top(14).Left(14).Bottom(14).Width(326).
		Radius(18).Material(glass.Glass{Tint: pal.glassTint, Clarity: 0.18}).
		Border(1, pal.panelBorder).Clip()

	panel.Children(func() {
		// 1. Titlebar Row (traffic lights inset on left; draggable window strip)
		ui.Row(c).Height(36).Padding(0, 10, 0, 82).Gap(4).Justify(ui.End).AlignItems(ui.Center).
			DragWindow().Children(func() {
			// EN / 中 language toggle
			langBtn := ui.ButtonBase(c).Row().Height(24).Padding(0, 8).Radius(7).Center().Cursor(ui.CursorPointer).
				Label("Toggle Language")
			if langBtn.Hovered() {
				langBtn.Background(pal.surfaceHover)
			}
			langBtn.Children(func() {
				ui.Text(c, locales.LangBtn()).FontSize(11).FontWeight(650).TextColor(pal.textSub)
			})
			if langBtn.Clicked() {
				next := "en"
				if a.svc.GetLocale() != "zh-Hans" {
					next = "zh-Hans"
				}
				go func() {
					a.svc.SetLocale(next)
					a.invalidate()
				}()
			}

			// Theme toggle
			themeBtn := ui.ButtonBase(c).Row().Size(26, 26).Radius(7).Center().Cursor(ui.CursorPointer).
				Label("Toggle Theme")
			if themeBtn.Hovered() {
				themeBtn.Background(pal.surfaceHover)
			}
			themeBtn.Children(func() {
				if pal.dark {
					ui.Icon(c, icSun).Size(14, 14).TextColor(pal.textSub)
				} else {
					ui.Icon(c, icMoon).Size(14, 14).TextColor(pal.textSub)
				}
			})
			if themeBtn.Clicked() {
				if pal.dark {
					mygo.Theme.SetSource(mygo.ThemeLight)
				} else {
					mygo.Theme.SetSource(mygo.ThemeDark)
				}
			}

			// Collapse sidebar button
			colBtn := ui.ButtonBase(c).Row().Size(26, 26).Radius(7).Center().Cursor(ui.CursorPointer).
				Label("Collapse Panel")
			if colBtn.Hovered() {
				colBtn.Background(pal.surfaceHover)
			}
			colBtn.Children(func() {
				ui.Icon(c, icSidebar).Size(14, 14).TextColor(pal.textSub)
			})
			if colBtn.Clicked() {
				a.collapsed = true
			}
		})

		// 2. Brand Header & Live Connection Pill
		var selectedDev *Device
		for i := range a.devices {
			if a.devices[i].UDID == a.selectedUDID {
				selectedDev = &a.devices[i]
				break
			}
		}
		online := selectedDev != nil

		ui.Row(c).Padding(2, 16, 10, 16).Justify(ui.SpaceBetween).AlignItems(ui.Center).
			DragWindow().Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				if a.appIcon != nil {
					ui.Image(c, a.appIcon).Size(28, 28).Radius(7).Clip()
				}
				ui.Text(c, locales.AppTitle()).FontSize(15.5).FontWeight(650).TextColor(pal.text)
			})

			trLabel := locales.Offline()
			trIcon := icOffline
			if selectedDev != nil {
				switch selectedDev.Transport {
				case "usb+wifi":
					trLabel = "USB + Wi-Fi"
					trIcon = icUSB
				case "wifi":
					trLabel = "Wi-Fi"
					trIcon = icWifi
				default:
					trLabel = "USB"
					trIcon = icUSB
				}
			}
			pillBg := pal.surface
			pillFg := pal.muted
			pillBorder := pal.divider
			if online {
				pillBg = pal.greenSoft
				pillFg = pal.green
				pillBorder = pal.green.Alpha(0.32)
			}
			ui.Row(c).Height(24).Padding(0, 10).Gap(5).Radius(999).
				Background(pillBg).Border(1, pillBorder).AlignItems(ui.Center).Children(func() {
				ui.Box(c).Size(6, 6).Radius(99).Background(pillFg)
				ui.Icon(c, trIcon).Size(12, 12).TextColor(pillFg)
				ui.Text(c, trLabel).FontSize(11.5).FontWeight(600).TextColor(pillFg)
			})
		})

		// 3. Scrollable Main Controls
		ui.Scroll(c).Grow(1).MinHeight(0).Padding(0, 14, 12, 14).Children(func() {
			ui.Column(c).Gap(11).Children(func() {
				a.buildDeviceRow(c, pal, selectedDev)
				a.buildSearchSection(c, pal)
				a.buildTelemetryCard(c, pal)
				a.buildPlacesSection(c, pal)
			})
		})

		// 4. Docked Console Drawer
		a.buildConsoleDrawer(c, pal)
	})
}

func (a *nativeApp) buildDeviceRow(c *ui.Context, pal palette, selectedDev *Device) {
	ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
		sel := ui.SelectBase(c, &a.selectedUDID)
		devLabel := locales.ScanningDevices()
		if selectedDev != nil {
			tag := "USB"
			if selectedDev.Transport == "wifi" {
				tag = "Wi-Fi"
			} else if selectedDev.Transport == "usb+wifi" {
				tag = "USB+Wi-Fi"
			}
			ver := selectedDev.OSVersion
			if ver == "" {
				ver = "?"
			}
			devLabel = fmt.Sprintf("%s (iOS %s · %s)", selectedDev.Name, ver, tag)
		} else if a.devicesScanned {
			devLabel = locales.NoDevice()
		}

		sel.Trigger.Grow(1).MinWidth(0).Height(34).Padding(0, 10).Gap(7).Radius(10).
			Background(pal.inputBg).Border(1, pal.divider).AlignItems(ui.Center).Cursor(ui.CursorPointer)
		if sel.Trigger.Hovered() {
			sel.Trigger.Border(1, pal.accentBorder)
		}
		sel.Trigger.Children(func() {
			ui.Icon(c, icPhone).Size(13, 13).TextColor(pal.muted)
			fg := pal.muted
			if selectedDev != nil {
				fg = pal.text
			}
			ui.Text(c, devLabel).SingleLine().Grow(1).MinWidth(0).FontSize(12).FontWeight(500).TextColor(fg)
			ui.Icon(c, icChevUD).Size(12, 12).TextColor(pal.muted)
		})

		sel.Popup(func(pop ui.Element) {
			pop.Margin(4, 0, 0, 0).Padding(4).Radius(12).Material(glass.Glass{}).Border(1, pal.panelBorder)
			if len(a.devices) == 0 {
				ui.Box(c).Padding(8, 10).Children(func() {
					ui.Text(c, locales.NoDevice()).FontSize(12).TextColor(pal.muted)
				})
				return
			}
			for _, d := range a.devices {
				tag := "USB"
				if d.Transport == "wifi" {
					tag = "Wi-Fi"
				} else if d.Transport == "usb+wifi" {
					tag = "USB+Wi-Fi"
				}
				ver := d.OSVersion
				if ver == "" {
					ver = "?"
				}
				txt := fmt.Sprintf("%s (iOS %s · %s)", d.Name, ver, tag)
				item := sel.Item(d.UDID).Padding(7, 10).Radius(8).Cursor(ui.CursorPointer)
				fg := pal.text
				if item.Highlighted() {
					item.Background(pal.accent)
					fg = ui.RGB(255, 255, 255)
				}
				item.Children(func() {
					ui.Text(c, txt).FontSize(12).TextColor(fg)
				})
			}
		})

		// Wi-Fi Pairing trigger
		pairBtn := ui.ButtonBase(c).Row().Size(34, 34).Radius(10).Background(pal.inputBg).Border(1, pal.divider).
			Center().Cursor(ui.CursorPointer).Label("Wi-Fi Pairing")
		if pairBtn.Hovered() {
			pairBtn.Background(pal.surfaceHover)
		}
		pairBtn.Children(func() {
			ui.Icon(c, icWifi).Size(14, 14).TextColor(pal.textSub)
		})
		if pairBtn.Clicked() {
			a.openPairing()
		}

		// Refresh devices button
		refBtn := ui.ButtonBase(c).Row().Size(34, 34).Radius(10).Background(pal.inputBg).Border(1, pal.divider).
			Center().Cursor(ui.CursorPointer).Label("Refresh Devices")
		if refBtn.Hovered() {
			refBtn.Background(pal.surfaceHover)
		}
		refBtn.Children(func() {
			ic := ui.Icon(c, icRefresh).Size(14, 14).TextColor(pal.textSub)
			if a.scanning {
				ic.Rotate(ic.Loop("scan-spin", 900*time.Millisecond, ui.Linear) * 360)
			}
		})
		if refBtn.Clicked() {
			a.refreshDevices()
		}
	})
}

func (a *nativeApp) buildSearchSection(c *ui.Context, pal palette) {
	ui.Column(c).Gap(5).Children(func() {
		sf := ui.SearchField(c, &a.searchQuery).
			Bind(&a.searchHandle).
			Placeholder(locales.SearchPlaceholder()).
			Height(34).Radius(10).Background(pal.inputBg).Border(1, pal.divider).FontSize(12.5)
		if sf.Changed() && strings.TrimSpace(a.searchQuery) == "" {
			a.searchOpen = false
			a.searchResults = nil
			a.searchErr = ""
		}
		if sf.Submitted() {
			a.runSearch(a.searchQuery)
		}

		if a.searchOpen {
			ui.Column(c).Padding(4).Gap(2).Radius(11).
				Background(pal.cardBg).Border(1, pal.panelBorder).Children(func() {
				if a.searchBusy {
					ui.Box(c).Padding(8, 10).Children(func() {
						ui.Text(c, locales.Searching()).FontSize(12).TextColor(pal.muted)
					})
					return
				}
				if a.searchErr != "" {
					ui.Box(c).Padding(8, 10).Children(func() {
						ui.Text(c, a.searchErr).FontSize(11.5).TextColor(pal.red)
					})
					return
				}
				if len(a.searchResults) == 0 {
					ui.Box(c).Padding(8, 10).Children(func() {
						ui.Text(c, locales.SearchEmpty()).FontSize(12).TextColor(pal.muted)
					})
					return
				}
				for _, it := range a.searchResults {
					row := ui.Row(c).Padding(7, 9).Gap(8).Radius(8).AlignItems(ui.Center).Cursor(ui.CursorPointer)
					if row.Hovered() {
						row.Background(pal.surfaceHover)
					}
					sub := it.Sub
					if sub == "" {
						sub = fmt.Sprintf("%.5f, %.5f", it.Latitude, it.Longitude)
					}
					row.Children(func() {
						ui.Icon(c, icPin).Size(13, 13).TextColor(pal.accent)
						ui.Column(c).Grow(1).MinWidth(0).Gap(1).Children(func() {
							ui.Text(c, it.Name).FontSize(12).FontWeight(550).TextColor(pal.text)
							ui.Text(c, sub).FontSize(10.5).TextColor(pal.muted)
						})
					})
					if row.Clicked() {
						a.jumpToWGS(it.Latitude, it.Longitude, 15)
						a.searchOpen = false
					}
				}
			})
		}
	})
}

func (a *nativeApp) buildTelemetryCard(c *ui.Context, pal palette) {
	dotCol := pal.muted
	title := locales.StatusIdle()
	if a.state.Busy {
		dotCol = pal.amber
		title = locales.StatusSyncingTitle()
	} else if a.state.Active {
		dotCol = pal.green
		if a.state.Label != "" {
			title = locales.StatusLockedPrefix(a.state.Label)
		} else {
			title = locales.StatusLocked()
		}
	} else if a.state.Error != "" {
		dotCol = pal.red
	}

	methodLabel := ""
	if a.state.Active && a.state.Method != "" {
		switch a.state.Method {
		case "devicectl":
			methodLabel = locales.MethodXcode()
		case "remotepairing":
			methodLabel = locales.MethodRemote()
		default:
			methodLabel = locales.MethodWifi()
		}
	}

	msg := a.state.Message
	if msg == "" {
		msg = locales.StatusHint()
	}

	ui.Column(c).Padding(12).Gap(10).Radius(14).
		Background(pal.cardBg).Border(1, pal.divider).Children(func() {
		// Status header row
		ui.Row(c).Justify(ui.SpaceBetween).AlignItems(ui.Center).Gap(8).Children(func() {
			ui.Row(c).Gap(7).AlignItems(ui.Center).Grow(1).MinWidth(0).Children(func() {
				ui.Box(c).Size(8, 8).Radius(99).Background(dotCol)
				ui.Text(c, title).FontSize(13).FontWeight(650).TextColor(pal.text)
			})
			if methodLabel != "" {
				ui.Box(c).Padding(2, 7).Radius(999).Background(pal.accentSoft).Children(func() {
					ui.Text(c, methodLabel).FontSize(10.5).FontWeight(600).TextColor(pal.accent)
				})
			}
		})

		// Coordinate wells + Copy button
		copied := !a.copiedUntil.IsZero() && c.Now().Before(a.copiedUntil)
		ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
			ui.Column(c).Grow(1).Padding(7, 10).Gap(2).Radius(9).Background(pal.inputBg).Children(func() {
				ui.Text(c, "LAT").FontSize(9.5).FontWeight(650).TextColor(pal.muted)
				ui.Text(c, fmt.Sprintf("%.5f", a.wgsLat)).Font("monospace").FontSize(12.5).FontWeight(600).TextColor(pal.text)
			})
			ui.Column(c).Grow(1).Padding(7, 10).Gap(2).Radius(9).Background(pal.inputBg).Children(func() {
				ui.Text(c, "LNG").FontSize(9.5).FontWeight(650).TextColor(pal.muted)
				ui.Text(c, fmt.Sprintf("%.5f", a.wgsLng)).Font("monospace").FontSize(12.5).FontWeight(600).TextColor(pal.text)
			})
			cpBtn := ui.ButtonBase(c).Row().Size(34, 44).Radius(9).Center().Cursor(ui.CursorPointer).Label("Copy Coordinates")
			if copied {
				cpBtn.Background(pal.greenSoft)
			} else if cpBtn.Hovered() {
				cpBtn.Background(pal.surfaceHover)
			} else {
				cpBtn.Background(pal.inputBg)
			}
			cpBtn.Children(func() {
				if copied {
					ui.Icon(c, icCheck).Size(14, 14).TextColor(pal.green)
				} else {
					ui.Icon(c, icCopy).Size(14, 14).TextColor(pal.textSub)
				}
			})
			if cpBtn.Clicked() {
				a.copyCoords(c)
			}
		})

		// Status message & optional error box
		ui.Text(c, msg).FontSize(11.5).TextColor(pal.muted)
		if a.state.Error != "" {
			ui.Box(c).Padding(6, 9).Radius(8).Background(pal.redSoft).Children(func() {
				ui.Text(c, a.state.Error).FontSize(11).TextColor(pal.red)
			})
		}

		// Action buttons row
		ui.Row(c).Gap(8).Children(func() {
			lockBtn := ui.ButtonBase(c).Row().Grow(1).Height(36).Radius(10).Gap(6).Center().Cursor(ui.CursorPointer).
				Background(pal.accent)
			if a.state.Busy {
				lockBtn.Opacity(0.6)
			} else if lockBtn.Hovered() {
				lockBtn.Background(pal.accent.Alpha(0.88))
			}
			lockBtn.Children(func() {
				ui.Icon(c, icTarget).Size(14, 14).TextColor(ui.RGB(255, 255, 255))
				ui.Text(c, locales.BtnLock()).FontSize(13).FontWeight(600).TextColor(ui.RGB(255, 255, 255))
			})
			if lockBtn.Clicked() && !a.state.Busy {
				lbl := fmt.Sprintf("%.4f, %.4f", a.wgsLat, a.wgsLng)
				a.lockLocation(c, a.wgsLat, a.wgsLng, lbl)
			}

			clrBg := pal.surface
			clrFg := pal.textSub
			clrBorder := pal.divider
			if a.state.Active {
				clrBg = pal.redSoft
				clrFg = pal.red
				clrBorder = pal.red.Alpha(0.35)
			}
			clrBtn := ui.ButtonBase(c).Row().Grow(1).Height(36).Radius(10).Gap(6).Center().Cursor(ui.CursorPointer).
				Background(clrBg).Border(1, clrBorder)
			if a.state.Busy {
				clrBtn.Opacity(0.6)
			} else if clrBtn.Hovered() {
				clrBtn.Background(pal.surfaceHover)
			}
			clrBtn.Children(func() {
				ui.Icon(c, icReset).Size(14, 14).TextColor(clrFg)
				ui.Text(c, locales.BtnClear()).FontSize(13).FontWeight(550).TextColor(clrFg)
			})
			if clrBtn.Clicked() && !a.state.Busy {
				a.clearLocation()
			}
		})
	})
}

func (a *nativeApp) buildPlacesSection(c *ui.Context, pal palette) {
	ui.Column(c).Gap(9).Children(func() {
		// Segmented tabs + "+ Save"
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			seg := ui.SegmentedBase(c, &a.tab, 2)
			seg.Track.Grow(1).Padding(3).Radius(10).Background(pal.inputBg).Gap(2).Children(func() {
				s0 := seg.Segment(0).Grow(1).Height(26).Radius(7.5).Center().Cursor(ui.CursorPointer)
				fg0 := pal.muted
				if a.tab == 0 {
					s0.Background(pal.surfaceHover)
					fg0 = pal.text
				}
				s0.Children(func() {
					ui.Text(c, locales.TabCities()).FontSize(11.5).FontWeight(600).TextColor(fg0)
				})

				s1 := seg.Segment(1).Grow(1).Height(26).Radius(7.5).Center().Gap(4).Cursor(ui.CursorPointer)
				fg1 := pal.muted
				if a.tab == 1 {
					s1.Background(pal.surfaceHover)
					fg1 = pal.text
				}
				s1.Children(func() {
					ui.Text(c, locales.TabFav()).FontSize(11.5).FontWeight(600).TextColor(fg1)
					ui.Box(c).Padding(0, 5).Radius(999).Background(pal.surface).Children(func() {
						ui.Text(c, fmt.Sprint(len(a.favorites))).FontSize(10).TextColor(pal.muted)
					})
				})
			})

			addBtn := ui.ButtonBase(c).Row().Height(32).Padding(0, 10).Gap(4).Radius(9).
				Background(pal.surface).Border(1, pal.divider).Center().Cursor(ui.CursorPointer)
			if addBtn.Hovered() {
				addBtn.Background(pal.surfaceHover)
			}
			addBtn.Children(func() {
				ui.Icon(c, icPlus).Size(12, 12).TextColor(pal.textSub)
				ui.Text(c, locales.BtnAddFav()).FontSize(11.5).FontWeight(600).TextColor(pal.textSub)
			})
			if addBtn.Clicked() {
				a.tab = 1
				a.favFormOpen = true
				a.favName = ""
			}
		})

		// Inline add favorite form
		if a.favFormOpen {
			ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
				inp := ui.TextInput(c, &a.favName).
					Placeholder(locales.FavPlaceholder()).
					AutoFocus().Grow(1).Height(30).Radius(8).
					Background(pal.inputBg).Border(1, pal.accentBorder).FontSize(12)
				if inp.Submitted() {
					a.saveFavorite(c)
				}
				saveBtn := ui.Row(c).Height(30).Padding(0, 10).Radius(8).Background(pal.accent).Center().Cursor(ui.CursorPointer).
					Children(func() {
						ui.Text(c, locales.BtnSave()).FontSize(11.5).FontWeight(600).TextColor(ui.RGB(255, 255, 255))
					})
				if saveBtn.Clicked() {
					a.saveFavorite(c)
				}
				cancelBtn := ui.Row(c).Height(30).Padding(0, 8).Radius(8).Background(pal.surface).Center().Cursor(ui.CursorPointer).
					Children(func() {
						ui.Text(c, locales.BtnCancel()).FontSize(11.5).TextColor(pal.muted)
					})
				if cancelBtn.Clicked() {
					a.favFormOpen = false
					a.favName = ""
				}
			})
		}

		// Tab contents
		if a.tab == 0 {
			ui.Column(c).Gap(7).Children(func() {
				for i := 0; i < len(presetCities); i += 2 {
					ui.Row(c).Gap(7).Children(func() {
						for j := i; j < min(i+2, len(presetCities)); j++ {
							p := presetCities[j]
							card := ui.ButtonBase(c).Row().Grow(1).Padding(10).Gap(6).Radius(11).
								Background(pal.cardBg).Border(1, pal.divider).
								Justify(ui.SpaceBetween).AlignItems(ui.Center).Cursor(ui.CursorPointer)
							if card.Hovered() {
								card.Background(pal.surfaceHover).Border(1, pal.accentBorder)
							}
							card.Children(func() {
								ui.Column(c).Gap(2).Children(func() {
									ui.Text(c, p.name()).FontSize(12.5).FontWeight(600).TextColor(pal.text)
									ui.Text(c, fmt.Sprintf("%.2f, %.2f", p.lat, p.lng)).
										Font("monospace").FontSize(10).TextColor(pal.muted)
								})
								ui.Box(c).Padding(2, 6).Radius(5).Background(pal.inputBg).Children(func() {
									ui.Text(c, p.region).FontSize(9.5).FontWeight(650).TextColor(pal.muted)
								})
							})
							if card.Clicked() {
								a.jumpToWGS(p.lat, p.lng, 13)
							}
						}
					})
				}
			})
		} else {
			if len(a.favorites) == 0 {
				ui.Box(c).Padding(14).Radius(11).Background(pal.cardBg).Border(1, pal.divider).Children(func() {
					ui.Text(c, locales.FavEmpty()).FontSize(11.5).TextColor(pal.muted)
				})
			} else {
				ui.Column(c).Gap(6).Children(func() {
					for _, fav := range a.favorites {
						f := fav
						row := ui.Row(c).Padding(8, 10).Gap(8).Radius(11).
							Background(pal.cardBg).Border(1, pal.divider).
							Justify(ui.SpaceBetween).AlignItems(ui.Center).Cursor(ui.CursorPointer)
						if row.Hovered() {
							row.Background(pal.surfaceHover)
						}
						var tpClicked, delClicked bool
						row.Children(func() {
							ui.Column(c).Grow(1).MinWidth(0).Gap(2).Children(func() {
								ui.Text(c, f.Name).FontSize(12.5).FontWeight(600).TextColor(pal.text)
								ui.Text(c, fmt.Sprintf("%.5f, %.5f", f.Latitude, f.Longitude)).
									Font("monospace").FontSize(10).TextColor(pal.muted)
							})
							ui.Row(c).Gap(5).AlignItems(ui.Center).Children(func() {
								tp := ui.Row(c).Height(24).Padding(0, 8).Gap(3).Radius(7).
									Background(pal.accent).Center().Cursor(ui.CursorPointer).Children(func() {
									ui.Icon(c, icTeleport).Size(10, 10).TextColor(ui.RGB(255, 255, 255))
									ui.Text(c, locales.FavTeleport()).FontSize(11).FontWeight(600).TextColor(ui.RGB(255, 255, 255))
								})
								tpClicked = tp.Clicked()

								del := ui.Row(c).Size(24, 24).Radius(7).Background(pal.inputBg).Center().Cursor(ui.CursorPointer)
								delFg := pal.muted
								if del.Hovered() {
									del.Background(pal.redSoft)
									delFg = pal.red
								}
								del.Children(func() {
									ui.Icon(c, icTrash).Size(11, 11).TextColor(delFg)
								})
								delClicked = del.Clicked()
							})
						})
						if tpClicked {
							a.jumpToWGS(f.Latitude, f.Longitude, 15)
							a.lockLocation(c, f.Latitude, f.Longitude, f.Name)
						} else if delClicked {
							_, _ = a.svc.DeleteFavorite(f.ID)
							a.reloadFavorites()
							c.Toast(locales.FavDeleted())
						} else if row.Clicked() {
							a.jumpToWGS(f.Latitude, f.Longitude, 15)
						}
					}
				})
			}
		}
	})
}

func (a *nativeApp) buildConsoleDrawer(c *ui.Context, pal palette) {
	ui.Column(c).BorderWidth(1, 0, 0, 0).BorderColor(pal.divider).Children(func() {
		toggle := ui.Row(c).Height(36).Padding(0, 14).Justify(ui.SpaceBetween).AlignItems(ui.Center).Cursor(ui.CursorPointer)
		if toggle.Hovered() {
			toggle.Background(pal.surface)
		}
		toggle.Children(func() {
			ui.Row(c).Gap(7).AlignItems(ui.Center).Children(func() {
				ui.Icon(c, icTerminal).Size(13, 13).TextColor(pal.muted)
				ui.Text(c, locales.LogsTitle()).FontSize(11.5).FontWeight(600).TextColor(pal.muted)
				ui.Box(c).Padding(0, 6).Radius(999).Background(pal.surface).Children(func() {
					ui.Text(c, fmt.Sprint(len(a.state.Logs))).FontSize(10).TextColor(pal.muted)
				})
			})
			if a.logOpen {
				ui.Icon(c, icChevronDn).Size(13, 13).TextColor(pal.muted)
			} else {
				ui.Icon(c, icChevronUp).Size(13, 13).TextColor(pal.muted)
			}
		})
		if toggle.Clicked() {
			a.logOpen = !a.logOpen
		}

		if a.logOpen {
			text := locales.LogsEmpty()
			if len(a.state.Logs) > 0 {
				start := max(len(a.state.Logs)-10, 0)
				text = strings.Join(a.state.Logs[start:], "\n")
			}
			ui.Box(c).Padding(0, 12, 12, 12).Children(func() {
				ui.Scroll(c).MaxHeight(120).Padding(8, 10).Radius(10).
					Background(pal.logBg).Border(1, pal.divider).Children(func() {
					ui.Text(c, text).Font("monospace").FontSize(10.5).TextColor(pal.textSub)
				})
			})
		}
	})
}

func (a *nativeApp) buildLayerBar(c *ui.Context, pal palette) {
	ui.Row(c).Absolute().Top(14).Right(14).Padding(3.5).Gap(2).Radius(12).
		Material(glass.Glass{Tint: pal.glassPill}).Border(1, pal.panelBorder).
		Children(func() {
			for _, layer := range allMapLayers {
				l := layer
				active := l == a.layer
				btn := ui.ButtonBase(c).Row().Height(27).Padding(0, 10).Radius(8.5).Center().Cursor(ui.CursorPointer)
				fg := pal.textSub
				if active {
					btn.Background(pal.accent)
					fg = ui.RGB(255, 255, 255)
				} else if btn.Hovered() {
					btn.Background(pal.surfaceHover)
					fg = pal.text
				}
				btn.Children(func() {
					ui.Text(c, l.label()).FontSize(12).FontWeight(600).TextColor(fg)
				})
				if btn.Clicked() {
					a.layer = l
					if maxZ := float64(l.maxZoom()); a.zoom > maxZ {
						a.zoom = maxZ
					}
				}
			}
		})
}

func (a *nativeApp) buildMapControls(c *ui.Context, pal palette) {
	ui.Column(c).Absolute().Bottom(36).Right(14).Gap(8).
		Children(func() {
			// Recenter button
			rec := ui.ButtonBase(c).Row().Size(36, 36).Radius(12).Material(glass.Glass{Tint: pal.glassPill, Interactive: true}).
				Border(1, pal.panelBorder).Center().Cursor(ui.CursorPointer).Label("Recenter")
			recFg := pal.text
			if a.state.Active {
				recFg = pal.accent
			}
			rec.Children(func() {
				ui.Icon(c, icTeleport).Size(15, 15).TextColor(recFg)
			})
			if rec.Clicked() {
				if a.state.Active {
					a.jumpToWGS(a.state.Latitude, a.state.Longitude, math.Max(a.zoom, 15))
					c.Toast(locales.RecenterOk())
				} else {
					c.Toast(locales.RecenterNone())
				}
			}

			// Zoom +/- stack
			ui.Column(c).Radius(12).Material(glass.Glass{Tint: pal.glassPill}).Border(1, pal.panelBorder).Clip().Children(func() {
				zi := ui.ButtonBase(c).Row().Size(36, 36).Center().Cursor(ui.CursorPointer).Label("Zoom In")
				if zi.Hovered() {
					zi.Background(pal.surfaceHover)
				}
				zi.Children(func() {
					ui.Icon(c, icPlus).Size(15, 15).TextColor(pal.text)
				})
				if zi.Clicked() {
					a.zoom = math.Min(a.zoom+1, float64(a.layer.maxZoom()))
				}

				ui.Box(c).Height(1).FillWidth().Background(pal.divider)

				zo := ui.ButtonBase(c).Row().Size(36, 36).Center().Cursor(ui.CursorPointer).Label("Zoom Out")
				if zo.Hovered() {
					zo.Background(pal.surfaceHover)
				}
				zo.Children(func() {
					ui.Icon(c, icMinus).Size(15, 15).TextColor(pal.text)
				})
				if zo.Clicked() {
					a.zoom = math.Max(a.zoom-1, 2.5)
				}
			})
		})
}

func (a *nativeApp) buildBottomHUD(c *ui.Context, pal palette) {
	ui.Row(c).Absolute().Bottom(16).Left(0).Right(0).Justify(ui.Center).PassThrough().Children(func() {
		hud := ui.ButtonBase(c).Row().Height(32).Padding(0, 13).Gap(8).Radius(999).
			Material(glass.Glass{Tint: pal.glassPill, Interactive: true}).Border(1, pal.panelBorder).
			AlignItems(ui.Center).Cursor(ui.CursorPointer).Label("Copy WGS-84 Coordinates")
		hud.Children(func() {
			ui.Box(c).Padding(2, 6).Radius(999).Background(pal.accentSoft).Children(func() {
				ui.Text(c, "WGS-84").FontSize(9.5).FontWeight(700).TextColor(pal.accent)
			})
			ui.Text(c, fmt.Sprintf("%.5f, %.5f", a.wgsLat, a.wgsLng)).
				Font("monospace").FontSize(12).FontWeight(600).TextColor(pal.text)
			ui.Box(c).Size(1, 13).Background(pal.divider)
			ui.Text(c, a.layer.label()).FontSize(11.5).FontWeight(550).TextColor(pal.textSub)
		})
		if hud.Clicked() {
			a.copyCoords(c)
		}
	})
}

func (a *nativeApp) buildPairingDialog(c *ui.Context, pal palette) {
	dlg := ui.DialogBase(c, &a.pairOpen, func(backdrop, panel ui.Element) {
		backdrop.Background(ui.RGBA(0, 0, 0, 0.38))
		panel.Width(384).Padding(20).Gap(14).Radius(20).
			Material(glass.Glass{Clarity: 0.2}).Border(1, pal.panelBorder)

		host := a.pairing.HostName
		if host == "" {
			host = "MacBook-Location"
		}

		// Header
		ui.Row(c).Justify(ui.SpaceBetween).AlignItems(ui.Center).Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				if a.appIcon != nil {
					ui.Image(c, a.appIcon).Size(30, 30).Radius(8).Clip()
				}
				ui.Column(c).Gap(2).Children(func() {
					ui.Text(c, locales.PairTitle()).FontSize(15).FontWeight(650).TextColor(pal.text)
					ui.Text(c, locales.PairSubtitle()).FontSize(11).TextColor(pal.muted)
				})
			})
			cls := ui.Row(c).Size(26, 26).Radius(7).Center().Cursor(ui.CursorPointer)
			if cls.Hovered() {
				cls.Background(pal.surfaceHover)
			}
			cls.Children(func() {
				ui.Icon(c, icClose).Size(14, 14).TextColor(pal.muted)
			})
			if cls.Clicked() {
				a.closePairing()
			}
		})

		// Steps
		ui.Column(c).Padding(12).Gap(6).Radius(12).Background(pal.cardBg).Border(1, pal.divider).Children(func() {
			ui.Text(c, locales.PairStep1()).FontSize(12).TextColor(pal.textSub)
			ui.Text(c, locales.PairStep2()).FontSize(12).TextColor(pal.textSub)
			ui.Text(c, locales.PairStep3(host)).FontSize(12).TextColor(pal.textSub)
		})

		// 6-digit PIN display
		if a.pairing.PIN != "" {
			pin := a.pairing.PIN
			if len(pin) == 6 {
				pin = pin[:3] + " " + pin[3:]
			}
			ui.Column(c).Padding(16, 12).Gap(6).Radius(14).
				Background(pal.accentSoft).Border(1, pal.accentBorder).
				AlignItems(ui.Center).Children(func() {
				ui.Text(c, locales.PairPinLabel()).FontSize(11).FontWeight(600).TextColor(pal.accent)
				ui.Text(c, pin).Font("monospace").FontSize(30).FontWeight(700).TextColor(pal.text)
			})
		}

		// Live status text
		statusTxt := a.pairing.Message
		statusCol := pal.muted
		if a.pairing.Error != "" {
			statusTxt = a.pairing.Error
			statusCol = pal.red
		} else if a.pairing.Status == "paired" {
			statusCol = pal.green
		} else if statusTxt == "" {
			statusTxt = locales.PairWaiting(host)
		}
		ui.Row(c).Justify(ui.Center).Children(func() {
			ui.Text(c, statusTxt).FontSize(12).TextColor(statusCol)
		})

		// Action buttons
		ui.Row(c).Gap(8).Children(func() {
			restart := ui.Row(c).Grow(1).Height(36).Radius(10).Background(pal.surface).
				Border(1, pal.divider).Center().Cursor(ui.CursorPointer)
			if restart.Hovered() {
				restart.Background(pal.surfaceHover)
			}
			restart.Children(func() {
				ui.Text(c, locales.PairRestart()).FontSize(12.5).FontWeight(550).TextColor(pal.text)
			})
			if restart.Clicked() {
				a.openPairing()
			}

			cancel := ui.Row(c).Grow(1).Height(36).Radius(10).Background(pal.surface).
				Border(1, pal.divider).Center().Cursor(ui.CursorPointer)
			if cancel.Hovered() {
				cancel.Background(pal.surfaceHover)
			}
			cancel.Children(func() {
				ui.Text(c, locales.BtnCancel()).FontSize(12.5).FontWeight(550).TextColor(pal.textSub)
			})
			if cancel.Clicked() {
				a.closePairing()
			}
		})
	})
	if dlg.Dismissed() {
		a.closePairing()
	}
}
