package main

import (
	"testing"

	"ios-location-mygo/locales"
)

func TestValidateCoords(t *testing.T) {
	if err := validateCoords(34.0537, -118.2428); err != nil {
		t.Fatalf("expected valid coords, got %v", err)
	}
	if err := validateCoords(91, 0); err == nil {
		t.Fatal("expected error for lat > 90")
	}
	if err := validateCoords(0, -181); err == nil {
		t.Fatal("expected error for lng < -180")
	}
}

func TestParseCoordQuery(t *testing.T) {
	lat, lng, ok := parseCoordQuery("35.65858, 139.74543")
	if !ok || lat != 35.65858 || lng != 139.74543 {
		t.Fatalf("unexpected parse result: %v %v %v", lat, lng, ok)
	}
	lat, lng, ok = parseCoordQuery("34.0537 -118.2428")
	if !ok || lat != 34.0537 || lng != -118.2428 {
		t.Fatalf("unexpected space-separated parse: %v %v %v", lat, lng, ok)
	}
	if _, _, ok := parseCoordQuery("Tokyo Tower"); ok {
		t.Fatal("expected non-coordinate text not to match")
	}
}

func TestMergeDevices(t *testing.T) {
	usbmux := []usbmuxEntry{
		{
			UniqueDeviceID: "UDID-1",
			DeviceName:     "Test iPhone",
			ProductType:    "iPhone18,2",
			ProductVersion: "27.2.1",
			ConnectionType: "Network",
		},
	}
	devctl := []devicectlEntry{
		{
			UDID:      "UDID-1",
			Name:      "Test iPhone",
			Model:     "iPhone 17 Pro Max",
			OSVersion: "27.2.1",
			Connected: false,
		},
		{
			UDID:      "UDID-2",
			Name:      "Offline iPhone",
			Connected: false,
		},
	}
	devs := mergeDevices(usbmux, devctl, nil)
	if len(devs) != 1 {
		t.Fatalf("expected 1 active device, got %d", len(devs))
	}
	if devs[0].Transport != "wifi" || devs[0].Model != "iPhone 17 Pro Max" || !devs[0].CanUserspace || devs[0].CanDevicectl {
		t.Fatalf("unexpected merged device: %+v", devs[0])
	}

	// RemotePairing Wi-Fi device discovered even when usbmux is empty.
	remoteOnly := mergeDevices(nil, devctl, []remoteWifiEntry{{Address: "192.0.2.10", Port: 49152, Identifier: "UDID-1"}})
	if len(remoteOnly) != 1 || !remoteOnly[0].CanRemotePairing || remoteOnly[0].Name != "Test iPhone" || remoteOnly[0].Transport != "wifi" {
		t.Fatalf("unexpected remote-only merged device: %+v", remoteOnly)
	}
}

func TestParsePairPin(t *testing.T) {
	pin, ok := parsePairPin("  Enter this code on your device: 482910")
	if !ok || pin != "482910" {
		t.Fatalf("expected pin 482910, got %q ok=%v", pin, ok)
	}
	if _, ok := parsePairPin("Waiting for a device..."); ok {
		t.Fatal("expected non-pin line not to match")
	}
}

func TestFavoritesCRUD(t *testing.T) {
	svc := NewSpooferService()
	svc.dataDir = t.TempDir()

	fav, err := svc.AddFavorite("东京塔", 35.65858, 139.74543)
	if err != nil {
		t.Fatalf("AddFavorite failed: %v", err)
	}
	list, err := svc.ListFavorites()
	if err != nil || len(list) != 1 || list[0].Name != "东京塔" {
		t.Fatalf("unexpected list: %+v, err=%v", list, err)
	}
	after, err := svc.DeleteFavorite(fav.ID)
	if err != nil || len(after) != 0 {
		t.Fatalf("expected empty after delete, got %+v err=%v", after, err)
	}
}

func TestLocaleNormalizationAndCatalog(t *testing.T) {
	cases := map[string]string{
		"zh":         "zh-Hans",
		"zh-CN":      "zh-Hans",
		"zh_Hans_CN": "zh-Hans",
		"en":         "en",
		"en-US":      "en",
		"":           "en",
	}
	for in, want := range cases {
		if got := normalizeLocale(in); got != want {
			t.Fatalf("normalizeLocale(%q) = %q, want %q", in, got, want)
		}
	}
	if got := locales.In("en").StatusCleared(); got != "Real GPS location restored" {
		t.Fatalf("unexpected en StatusCleared: %q", got)
	}
	if got := locales.In("zh-Hans").StatusCleared(); got != "已恢复真实定位" {
		t.Fatalf("unexpected zh-Hans StatusCleared: %q", got)
	}
}

func TestGCJAndMercatorRoundtrip(t *testing.T) {
	// Overseas coordinates remain unchanged.
	glat, glng := wgsToGCJ(34.0537, -118.2428)
	if glat != 34.0537 || glng != -118.2428 {
		t.Fatalf("expected overseas coords unchanged, got %v, %v", glat, glng)
	}
	// Beijing coordinates offset and roundtrip within 1e-4 deg.
	glat, glng = wgsToGCJ(39.90872, 116.39748)
	if glat == 39.90872 {
		t.Fatal("expected GCJ-02 offset in Beijing")
	}
	wlat, wlng := gcjToWGS(glat, glng)
	if diff := wlat - 39.90872; diff < -1e-4 || diff > 1e-4 {
		t.Fatalf("unexpected GCJ roundtrip lat: %v", wlat)
	}
	if diff := wlng - 116.39748; diff < -1e-4 || diff > 1e-4 {
		t.Fatalf("unexpected GCJ roundtrip lng: %v", wlng)
	}
	// Web Mercator projection roundtrip.
	x, y := latLngToWorldPx(34.0536923, -118.2427676, 13)
	rlat, rlng := worldPxToLatLng(x, y, 13)
	if diff := rlat - 34.0536923; diff < -1e-6 || diff > 1e-6 {
		t.Fatalf("unexpected mercator lat: %v", rlat)
	}
	if diff := rlng - (-118.2427676); diff < -1e-6 || diff > 1e-6 {
		t.Fatalf("unexpected mercator lng: %v", rlng)
	}
}
