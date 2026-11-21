package main

import "math"

const (
	earthA   = 6378245.0
	earthEE  = 0.00669342162296594323
	tileSize = 256.0
)

func outOfChina(lat, lng float64) bool {
	return lng < 72.004 || lng > 137.8347 || lat < 0.8293 || lat > 55.8271
}

func transformLat(x, y float64) float64 {
	r := -100.0 + 2.0*x + 3.0*y + 0.2*y*y + 0.1*x*y + 0.2*math.Sqrt(math.Abs(x))
	r += (20.0*math.Sin(6.0*x*math.Pi) + 20.0*math.Sin(2.0*x*math.Pi)) * 2.0 / 3.0
	r += (20.0*math.Sin(y*math.Pi) + 40.0*math.Sin(y/3.0*math.Pi)) * 2.0 / 3.0
	r += (160.0*math.Sin(y/12.0*math.Pi) + 320.0*math.Sin(y*math.Pi/30.0)) * 2.0 / 3.0
	return r
}

func transformLng(x, y float64) float64 {
	r := 300.0 + x + 2.0*y + 0.1*x*x + 0.1*x*y + 0.1*math.Sqrt(math.Abs(x))
	r += (20.0*math.Sin(6.0*x*math.Pi) + 20.0*math.Sin(2.0*x*math.Pi)) * 2.0 / 3.0
	r += (20.0*math.Sin(x*math.Pi) + 40.0*math.Sin(x/3.0*math.Pi)) * 2.0 / 3.0
	r += (150.0*math.Sin(x/12.0*math.Pi) + 300.0*math.Sin(x/30.0*math.Pi)) * 2.0 / 3.0
	return r
}

// wgsToGCJ converts WGS-84 coordinates to GCJ-02 (China Mars datum) for Google/AutoNavi China tiles.
func wgsToGCJ(lat, lng float64) (float64, float64) {
	if outOfChina(lat, lng) {
		return lat, lng
	}
	dLat := transformLat(lng-105.0, lat-35.0)
	dLng := transformLng(lng-105.0, lat-35.0)
	radLat := lat / 180.0 * math.Pi
	magic := math.Sin(radLat)
	magic = 1.0 - earthEE*magic*magic
	sqrtMagic := math.Sqrt(magic)
	dLat = (dLat * 180.0) / ((earthA * (1.0 - earthEE)) / (magic * sqrtMagic) * math.Pi)
	dLng = (dLng * 180.0) / (earthA / sqrtMagic * math.Cos(radLat) * math.Pi)
	return lat + dLat, lng + dLng
}

// gcjToWGS converts GCJ-02 coordinates back to WGS-84.
func gcjToWGS(lat, lng float64) (float64, float64) {
	if outOfChina(lat, lng) {
		return lat, lng
	}
	gLat, gLng := wgsToGCJ(lat, lng)
	return lat*2.0 - gLat, lng*2.0 - gLng
}

func clampMercatorLat(lat float64) float64 {
	if lat < -85.05112878 {
		return -85.05112878
	}
	if lat > 85.05112878 {
		return 85.05112878
	}
	return lat
}

func wrapLng(lng float64) float64 {
	x := math.Mod(lng+180.0, 360.0)
	if x < 0 {
		x += 360.0
	}
	return x - 180.0
}

func latLngToWorldPx(lat, lng, zoom float64) (float64, float64) {
	scale := tileSize * math.Pow(2.0, zoom)
	x := (lng + 180.0) / 360.0 * scale
	sinLat := math.Sin(clampMercatorLat(lat) * math.Pi / 180.0)
	y := (0.5 - math.Log((1.0+sinLat)/(1.0-sinLat))/(4.0*math.Pi)) * scale
	return x, y
}

func worldPxToLatLng(x, y, zoom float64) (float64, float64) {
	scale := tileSize * math.Pow(2.0, zoom)
	lng := wrapLng(x/scale*360.0 - 180.0)
	n := math.Pi - 2.0*math.Pi*(y/scale)
	lat := clampMercatorLat(math.Atan(math.Sinh(n)) * 180.0 / math.Pi)
	return lat, lng
}
