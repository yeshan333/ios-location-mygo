package main

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"sync"
	"time"

	"github.com/egoist/mygo/ui"
	"ios-location-mygo/locales"
)

type MapLayer int

const (
	LayerGoogleMaps MapLayer = iota
	LayerGoogleSat
	LayerEsriStreets
	LayerAutoNavi
	LayerAutoNaviSat
)

var allMapLayers = []MapLayer{
	LayerGoogleMaps,
	LayerGoogleSat,
	LayerEsriStreets,
	LayerAutoNavi,
	LayerAutoNaviSat,
}

func (l MapLayer) isGCJ() bool {
	return l != LayerEsriStreets
}

func (l MapLayer) maxZoom() int {
	switch l {
	case LayerGoogleMaps, LayerGoogleSat:
		return 20
	default:
		return 18
	}
}

func (l MapLayer) hasOverlay() bool {
	return l == LayerAutoNaviSat
}

func (l MapLayer) label() string {
	switch l {
	case LayerGoogleMaps:
		return locales.LayerGmap()
	case LayerGoogleSat:
		return locales.LayerGsat()
	case LayerEsriStreets:
		return locales.LayerOsm()
	case LayerAutoNavi:
		return locales.LayerAmap()
	case LayerAutoNaviSat:
		return locales.LayerSat()
	default:
		return ""
	}
}

func (l MapLayer) attribution() string {
	switch l {
	case LayerGoogleMaps:
		return "© Google Maps"
	case LayerGoogleSat:
		return "© Google Satellite"
	case LayerEsriStreets:
		return "© Esri World Street Map"
	default:
		return "© AutoNavi"
	}
}

func (l MapLayer) cachePrefix(lang string, overlay bool) string {
	switch l {
	case LayerGoogleMaps:
		return "gmap_" + lang
	case LayerGoogleSat:
		return "gsat_" + lang
	case LayerEsriStreets:
		return "esri"
	case LayerAutoNavi:
		return "amap"
	case LayerAutoNaviSat:
		if overlay {
			return "asat_lbl"
		}
		return "asat_img"
	default:
		return "other"
	}
}

func (l MapLayer) tileURL(lang string, z, x, y int, overlay bool) string {
	s4 := (absInt(x) + absInt(y)) % 4
	switch l {
	case LayerGoogleMaps:
		return fmt.Sprintf("https://mt%d.google.com/vt/lyrs=m&hl=%s&x=%d&y=%d&z=%d", s4, lang, x, y, z)
	case LayerGoogleSat:
		return fmt.Sprintf("https://mt%d.google.com/vt/lyrs=y&hl=%s&x=%d&y=%d&z=%d", s4, lang, x, y, z)
	case LayerEsriStreets:
		return fmt.Sprintf("https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/%d/%d/%d", z, y, x)
	case LayerAutoNavi:
		return fmt.Sprintf("https://wprd0%d.is.autonavi.com/appmaptile?x=%d&y=%d&z=%d&lang=zh_cn&size=1&scl=1&style=7", s4+1, x, y, z)
	case LayerAutoNaviSat:
		if overlay {
			return fmt.Sprintf("https://wprd0%d.is.autonavi.com/appmaptile?x=%d&y=%d&z=%d&lang=zh_cn&size=1&scl=1&style=8", s4+1, x, y, z)
		}
		return fmt.Sprintf("https://webst0%d.is.autonavi.com/appmaptile?style=6&x=%d&y=%d&z=%d", s4+1, x, y, z)
	default:
		return ""
	}
}

func absInt(v int) int {
	if v < 0 {
		return -v
	}
	return v
}

type tileKey struct {
	layer   MapLayer
	lang    string // "zh-CN" or "en" (only relevant for Google layers)
	overlay bool
	z, x, y int
}

func (k tileKey) canonical() tileKey {
	if k.layer != LayerGoogleMaps && k.layer != LayerGoogleSat {
		k.lang = ""
	}
	return k
}

func (k tileKey) diskPath(root string) string {
	return filepath.Join(
		root,
		k.layer.cachePrefix(k.lang, k.overlay),
		strconv.Itoa(k.z),
		strconv.Itoa(k.x),
		strconv.Itoa(k.y)+".tile",
	)
}

type TileManager struct {
	mu       sync.Mutex
	cond     *sync.Cond
	cache    map[tileKey]*ui.Bitmap
	order    []tileKey
	inflight map[tileKey]bool
	queue    []tileKey
	diskRoot string
	onReady  func()
}

const (
	maxMemTiles = 420
	tileWorkers = 6
)

func NewTileManager(onReady func()) *TileManager {
	root := filepath.Join(os.TempDir(), "ios-location-tiles")
	if cacheDir, err := os.UserCacheDir(); err == nil {
		root = filepath.Join(cacheDir, "iOS Location", "tiles")
	}
	_ = os.MkdirAll(root, 0o755)

	tm := &TileManager{
		cache:    make(map[tileKey]*ui.Bitmap, maxMemTiles),
		order:    make([]tileKey, 0, maxMemTiles),
		inflight: make(map[tileKey]bool),
		diskRoot: root,
		onReady:  onReady,
	}
	tm.cond = sync.NewCond(&tm.mu)

	client := &http.Client{Timeout: 10 * time.Second}
	for range tileWorkers {
		go tm.worker(client)
	}
	return tm
}

func (tm *TileManager) worker(client *http.Client) {
	for {
		tm.mu.Lock()
		for len(tm.queue) == 0 {
			tm.cond.Wait()
		}
		// LIFO pops the most recently visible viewport tiles first.
		last := len(tm.queue) - 1
		key := tm.queue[last]
		tm.queue = tm.queue[:last]
		tm.mu.Unlock()

		diskPath := key.diskPath(tm.diskRoot)
		data, err := os.ReadFile(diskPath)
		if err != nil || len(data) == 0 {
			url := key.layer.tileURL(key.lang, key.z, key.x, key.y, key.overlay)
			req, rerr := http.NewRequest(http.MethodGet, url, nil)
			if rerr == nil {
				req.Header.Set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15")
				if resp, cerr := client.Do(req); cerr == nil {
					if resp.StatusCode == http.StatusOK {
						if body, berr := io.ReadAll(resp.Body); berr == nil && len(body) > 0 {
							data = body
							_ = os.MkdirAll(filepath.Dir(diskPath), 0o755)
							_ = os.WriteFile(diskPath, body, 0o644)
						}
					}
					_ = resp.Body.Close()
				}
			}
		}

		var bm *ui.Bitmap
		if len(data) > 0 {
			bm, _ = ui.DecodeBitmap(data)
		}

		tm.mu.Lock()
		delete(tm.inflight, key)
		if bm != nil {
			tm.cache[key] = bm
			tm.order = append(tm.order, key)
			for len(tm.order) > maxMemTiles {
				old := tm.order[0]
				tm.order = tm.order[1:]
				delete(tm.cache, old)
			}
		}
		cb := tm.onReady
		tm.mu.Unlock()

		if bm != nil && cb != nil {
			cb()
		}
	}
}

// GetOrRequest returns the cached GPU bitmap for key, or schedules a background fetch.
func (tm *TileManager) GetOrRequest(key tileKey) *ui.Bitmap {
	key = key.canonical()
	tm.mu.Lock()
	defer tm.mu.Unlock()

	if bm, ok := tm.cache[key]; ok {
		return bm
	}
	if !tm.inflight[key] {
		tm.inflight[key] = true
		if len(tm.queue) > 160 {
			dropped := tm.queue[0]
			tm.queue = tm.queue[1:]
			delete(tm.inflight, dropped)
		}
		tm.queue = append(tm.queue, key)
		tm.cond.Signal()
	}
	return nil
}
