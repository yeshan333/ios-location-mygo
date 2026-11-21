package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/egoist/mygo"
	"ios-location-mygo/locales"
)

// StateEvent broadcasts live spoofer state changes to open windows.
var StateEvent = mygo.NewEvent[SpooferState]("spoofer:state")

// PairingEvent broadcasts live iOS 27+ Wi-Fi pairing status and 6-digit PIN.
var PairingEvent = mygo.NewEvent[PairingState]("spoofer:pairing")

// LocaleEvent broadcasts app language changes ("en" or "zh-Hans").
var LocaleEvent = mygo.NewEvent[string]("spoofer:locale")

// Device represents a connected iOS device discovered over Wi-Fi or USB.
type Device struct {
	UDID             string `json:"udid"`
	Name             string `json:"name"`
	Model            string `json:"model"`
	OSVersion        string `json:"osVersion"`
	Transport        string `json:"transport"` // "wifi", "usb", "usb+wifi"
	CanDevicectl     bool   `json:"canDevicectl"`
	CanRemotePairing bool   `json:"canRemotePairing"`
	RemoteIdentifier string `json:"remoteIdentifier,omitempty"`
	CanUserspace     bool   `json:"canUserspace"`
}

// PairingState describes an active iOS 27+ Wi-Fi remote pair-host session.
type PairingState struct {
	Active     bool   `json:"active"`
	HostName   string `json:"hostName,omitempty"`
	PIN        string `json:"pin,omitempty"`
	Status     string `json:"status"` // "idle", "waiting", "pin", "paired", "error"
	DeviceName string `json:"deviceName,omitempty"`
	DeviceUDID string `json:"deviceUdid,omitempty"`
	Message    string `json:"message,omitempty"`
	Error      string `json:"error,omitempty"`
}

// SetLocationRequest is the input for setting a simulated GPS coordinate.
type SetLocationRequest struct {
	UDID      string  `json:"udid"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
	Label     string  `json:"label,omitempty"`
}

// SpooferState describes the active location simulation session.
type SpooferState struct {
	Active    bool     `json:"active"`
	Busy      bool     `json:"busy"`
	UDID      string   `json:"udid,omitempty"`
	Latitude  float64  `json:"latitude,omitempty"`
	Longitude float64  `json:"longitude,omitempty"`
	Label     string   `json:"label,omitempty"`
	Method    string   `json:"method,omitempty"` // "devicectl" or "pymobiledevice3"
	Message   string   `json:"message,omitempty"`
	Error     string   `json:"error,omitempty"`
	Logs      []string `json:"logs"`
}

// Favorite is a user-saved location bookmark.
type Favorite struct {
	ID        string  `json:"id"`
	Name      string  `json:"name"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
	CreatedAt string  `json:"createdAt"`
}

// SearchResult is a geocoding match returned to the map UI.
type SearchResult struct {
	Name      string  `json:"name"`
	Sub       string  `json:"sub"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
	Direct    bool    `json:"direct,omitempty"`
}

// SpooferService exposes iOS device discovery, location simulation, favorites,
// and geocoding to the MyGo frontend.
type SpooferService struct {
	mu         sync.Mutex
	dataDir    string
	state      SpooferState
	active     *exec.Cmd
	activeDone chan struct{}
	activeID   uint64
	pairState  PairingState
	pairCmd    *exec.Cmd
	pairDone   chan struct{}
	pairID     uint64
	devices    map[string]Device
}

func NewSpooferService() *SpooferService {
	return &SpooferService{
		state:     SpooferState{Logs: []string{}},
		pairState: PairingState{Status: "idle"},
		devices:   make(map[string]Device),
	}
}

// InitDataDir resolves the user data directory after MyGo App is ready.
func (s *SpooferService) InitDataDir() {
	dir, err := mygo.App.Path(mygo.PathUserData)
	if err != nil || dir == "" {
		if cfg, cerr := os.UserConfigDir(); cerr == nil {
			dir = filepath.Join(cfg, "iOS Location")
		}
	}
	if dir != "" {
		_ = os.MkdirAll(dir, 0o755)
	}
	s.mu.Lock()
	s.dataDir = dir
	s.mu.Unlock()
}

func normalizeLocale(tag string) string {
	t := strings.ToLower(strings.TrimSpace(tag))
	if strings.HasPrefix(t, "zh") {
		return "zh-Hans"
	}
	return "en"
}

// GetLocale returns the active app language ("zh-Hans" or "en").
func (s *SpooferService) GetLocale() string {
	return normalizeLocale(mygo.App.Locale())
}

// SetLocale switches the app language (including native macOS menus) and persists it.
func (s *SpooferService) SetLocale(tag string) string {
	norm := normalizeLocale(tag)
	mygo.App.SetLocale(norm)
	// ChosenLocale synchronizes with the main thread queue so SetLocale's
	// asynchronous postMain finishes before we read the active locale back.
	_ = mygo.App.ChosenLocale()
	return normalizeLocale(mygo.App.Locale())
}

// GetState returns the current simulation state and recent logs.
func (s *SpooferService) GetState() SpooferState {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.cloneStateLocked()
}

func (s *SpooferService) cloneStateLocked() SpooferState {
	cp := s.state
	cp.Logs = append([]string(nil), s.state.Logs...)
	return cp
}

func (s *SpooferService) appendLogLocked(line string) {
	line = strings.TrimSpace(line)
	if line == "" {
		return
	}
	ts := time.Now().Format("15:04:05")
	s.state.Logs = append(s.state.Logs, fmt.Sprintf("[%s] %s", ts, line))
	if len(s.state.Logs) > 80 {
		s.state.Logs = s.state.Logs[len(s.state.Logs)-80:]
	}
}

func (s *SpooferService) broadcastState(st SpooferState) {
	_ = StateEvent.Broadcast(st)
}

// ListDevices discovers iOS devices via pymobiledevice3 remote browse (iOS 17+/27+
// RemotePairing over Wi-Fi, independent of usbmuxd), pymobiledevice3 usbmux, and
// Xcode's xcrun devicectl.
func (s *SpooferService) ListDevices(ctx context.Context) ([]Device, error) {
	var (
		wg          sync.WaitGroup
		usbmuxDevs  []usbmuxEntry
		devCtlDevs  []devicectlEntry
		remoteDevs  []remoteWifiEntry
		usbmuxErr   error
		devCtlErr   error
		remoteErr   error
	)

	wg.Add(3)
	go func() {
		defer wg.Done()
		usbmuxDevs, usbmuxErr = s.queryUsbmux(ctx)
	}()
	go func() {
		defer wg.Done()
		devCtlDevs, devCtlErr = s.queryDevicectl(ctx)
	}()
	go func() {
		defer wg.Done()
		remoteDevs, remoteErr = s.queryRemoteBrowse(ctx)
	}()
	wg.Wait()

	merged := mergeDevices(usbmuxDevs, devCtlDevs, remoteDevs)

	s.mu.Lock()
	s.devices = make(map[string]Device, len(merged))
	for _, d := range merged {
		s.devices[d.UDID] = d
	}
	if usbmuxErr != nil {
		s.appendLogLocked(locales.LogUsbmuxErr(usbmuxErr.Error()))
	}
	if remoteErr != nil {
		s.appendLogLocked(locales.LogRemoteErr(remoteErr.Error()))
	}
	if devCtlErr != nil {
		s.appendLogLocked(locales.LogDevicectlErr(devCtlErr.Error()))
	}
	s.appendLogLocked(locales.LogScanDone(strconv.Itoa(len(merged)), strconv.Itoa(len(remoteDevs)), strconv.Itoa(len(usbmuxDevs))))
	st := s.cloneStateLocked()
	s.mu.Unlock()
	s.broadcastState(st)

	if len(merged) == 0 && usbmuxErr != nil && devCtlErr != nil && remoteErr != nil {
		return []Device{}, errors.New(locales.ErrToolMissing(usbmuxErr.Error()))
	}
	return merged, nil
}

// SetLocation applies the requested WGS-84 coordinate to the selected iOS device.
func (s *SpooferService) SetLocation(ctx context.Context, req SetLocationRequest) (SpooferState, error) {
	if err := validateCoords(req.Latitude, req.Longitude); err != nil {
		return s.GetState(), err
	}
	udid := strings.TrimSpace(req.UDID)
	if udid == "" {
		devs, _ := s.ListDevices(ctx)
		if len(devs) == 0 {
			return s.GetState(), errors.New(locales.ErrNoDevice())
		}
		udid = devs[0].UDID
	}

	// Ensure we know device capabilities.
	s.mu.Lock()
	dev, ok := s.devices[udid]
	s.mu.Unlock()
	if !ok {
		_, _ = s.ListDevices(ctx)
		s.mu.Lock()
		dev = s.devices[udid]
		s.mu.Unlock()
	}

	label := strings.TrimSpace(req.Label)
	if label == "" {
		label = fmt.Sprintf("%.5f, %.5f", req.Latitude, req.Longitude)
	}

	// Stop any running background simulation process first.
	wasRunning := s.stopActiveProcess()

	s.mu.Lock()
	s.state.Busy = true
	s.state.Error = ""
	s.state.Message = locales.StatusSyncing()
	s.appendLogLocked(locales.LogPrepSet(label, fmt.Sprintf("%.6f", req.Latitude), fmt.Sprintf("%.6f", req.Longitude)))
	st := s.cloneStateLocked()
	s.mu.Unlock()
	s.broadcastState(st)

	// Give iOS a brief moment to tear down the previous RemotePairing tunnel slot.
	if wasRunning && dev.CanRemotePairing {
		time.Sleep(1500 * time.Millisecond)
	}

	// Prefer native xcrun devicectl when wired & connected (instant, no daemon needed).
	if dev.CanDevicectl {
		if err := s.runDevicectlSet(ctx, udid, req.Latitude, req.Longitude); err == nil {
			s.mu.Lock()
			s.state.Busy = false
			s.state.Active = true
			s.state.UDID = udid
			s.state.Latitude = req.Latitude
			s.state.Longitude = req.Longitude
			s.state.Label = label
			s.state.Method = "devicectl"
			s.state.Error = ""
			s.state.Message = locales.StatusActiveDevicectl()
			s.appendLogLocked(locales.LogDevicectlOk())
			st = s.cloneStateLocked()
			s.mu.Unlock()
			s.broadcastState(st)
			return st, nil
		} else {
			s.mu.Lock()
			s.appendLogLocked(locales.LogDevicectlFallback(err.Error()))
			s.mu.Unlock()
		}
	}

	// Use pymobiledevice3: prefer --remotepairing (direct Wi-Fi Bonjour, no usbmuxd!)
	// when paired via remote pair-host, else --userspace via usbmux.
	useRemotePairing := dev.CanRemotePairing
	targetID := udid
	if useRemotePairing && dev.RemoteIdentifier != "" {
		targetID = dev.RemoteIdentifier
	}
	err := s.startPymobiledevice3Set(udid, targetID, useRemotePairing, req.Latitude, req.Longitude, label)
	if err != nil && useRemotePairing && strings.Contains(err.Error(), "RemotePairing") {
		time.Sleep(2 * time.Second)
		err = s.startPymobiledevice3Set(udid, targetID, true, req.Latitude, req.Longitude, label)
	}
	if err != nil {
		s.mu.Lock()
		s.state.Busy = false
		s.state.Active = false
		s.state.Error = err.Error()
		s.state.Message = locales.StatusSetFailed()
		s.appendLogLocked(locales.LogSetFailed(err.Error()))
		st = s.cloneStateLocked()
		s.mu.Unlock()
		s.broadcastState(st)
		return st, err
	}

	return s.GetState(), nil
}

// ClearLocation restores the real GPS location on the device.
func (s *SpooferService) ClearLocation(ctx context.Context, udid string) (SpooferState, error) {
	s.mu.Lock()
	if strings.TrimSpace(udid) == "" {
		udid = s.state.UDID
	}
	dev := s.devices[udid]
	s.state.Busy = true
	s.state.Error = ""
	s.state.Message = locales.StatusClearing()
	s.appendLogLocked(locales.LogClearing())
	st := s.cloneStateLocked()
	s.mu.Unlock()
	s.broadcastState(st)

	wasRunning := s.stopActiveProcess()

	// Give iOS time to release the single RemotePairing tunnel before reconnecting for clear.
	if wasRunning && dev.CanRemotePairing {
		time.Sleep(1500 * time.Millisecond)
	}

	var clearErr error
	if udid != "" {
		if dev.CanDevicectl {
			clearErr = s.runDevicectlClear(ctx, udid)
		}
		if !dev.CanDevicectl || clearErr != nil {
			targetID := udid
			if dev.CanRemotePairing && dev.RemoteIdentifier != "" {
				targetID = dev.RemoteIdentifier
			}
			clearErr = s.runPymobiledevice3Clear(ctx, targetID, dev.CanRemotePairing)
			// Retry once if iOS RemotePairing was still releasing the previous session.
			if clearErr != nil && dev.CanRemotePairing && strings.Contains(clearErr.Error(), "RemotePairing") {
				time.Sleep(2 * time.Second)
				clearErr = s.runPymobiledevice3Clear(ctx, targetID, true)
			}
		}
	}

	s.mu.Lock()
	s.state.Busy = false
	s.state.Active = false
	s.state.Label = ""
	s.state.Method = ""
	if clearErr != nil {
		s.state.Error = clearErr.Error()
		s.state.Message = locales.StatusClearWithError(clearErr.Error())
		s.appendLogLocked(locales.LogClearErr(clearErr.Error()))
	} else {
		s.state.Error = ""
		s.state.Message = locales.StatusCleared()
		s.appendLogLocked(locales.LogCleared())
	}
	st = s.cloneStateLocked()
	s.mu.Unlock()
	s.broadcastState(st)
	return st, clearErr
}

// Shutdown terminates any active background processes before app exit.
func (s *SpooferService) Shutdown() {
	s.stopActiveProcess()
	s.StopWifiPairing()
}

func (s *SpooferService) stopActiveProcess() bool {
	s.mu.Lock()
	cmd := s.active
	done := s.activeDone
	s.active = nil
	s.activeDone = nil
	s.activeID++
	s.mu.Unlock()

	if cmd == nil || cmd.Process == nil {
		return false
	}
	_ = cmd.Process.Signal(syscall.SIGINT)
	if done != nil {
		select {
		case <-done:
		case <-time.After(2500 * time.Millisecond):
			_ = cmd.Process.Kill()
			select {
			case <-done:
			case <-time.After(1 * time.Second):
			}
		}
	} else {
		_ = cmd.Process.Kill()
	}
	return true
}

func (s *SpooferService) startPymobiledevice3Set(udid, targetID string, useRemotePairing bool, lat, lng float64, label string) error {
	bin, prefixArgs, err := findPymobiledevice3()
	if err != nil {
		return err
	}
	modeFlag := "--userspace"
	methodName := "pymobiledevice3"
	msgText := locales.StatusActiveUserspace()
	if useRemotePairing {
		modeFlag = "--remotepairing"
		methodName = "remotepairing"
		msgText = locales.StatusActiveRemotePairing()
	}
	args := append(prefixArgs,
		"developer", "dvt", "simulate-location", "set",
		modeFlag,
		"--udid", targetID,
		"--",
		strconv.FormatFloat(lat, 'f', 7, 64),
		strconv.FormatFloat(lng, 'f', 7, 64),
	)

	cmd := exec.Command(bin, args...)
	cmd.Env = augmentedEnv()
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return errors.New(locales.ErrStartFailed(err.Error()))
	}

	doneCh := make(chan struct{})
	s.mu.Lock()
	s.active = cmd
	s.activeDone = doneCh
	s.activeID++
	myID := s.activeID
	s.mu.Unlock()

	readyCh := make(chan struct{}, 1)
	errCh := make(chan error, 1)
	var outBuf sync.Mutex
	var lastLines []string

	readPipe := func(r io.Reader) {
		sc := bufio.NewScanner(r)
		for sc.Scan() {
			line := strings.TrimSpace(sc.Text())
			if line == "" {
				continue
			}
			outBuf.Lock()
			lastLines = append(lastLines, line)
			if len(lastLines) > 10 {
				lastLines = lastLines[len(lastLines)-10:]
			}
			outBuf.Unlock()

			s.mu.Lock()
			if s.activeID == myID {
				s.appendLogLocked(line)
			}
			s.mu.Unlock()

			if strings.Contains(line, "Press Ctrl+C") || strings.Contains(line, "SIGINT") {
				select {
				case readyCh <- struct{}{}:
				default:
				}
			}
		}
	}

	go readPipe(stdout)
	go readPipe(stderr)

	go func() {
		waitErr := cmd.Wait()
		close(doneCh)
		select {
		case errCh <- waitErr:
		default:
		}
		s.mu.Lock()
		if s.activeID == myID {
			s.active = nil
			s.activeDone = nil
			if s.state.Active {
				s.state.Active = false
				s.state.Message = locales.StatusDisconnected()
				if waitErr != nil {
					s.appendLogLocked(locales.LogSessionExited(waitErr.Error()))
				} else {
					s.appendLogLocked(locales.LogSessionEnded())
				}
				st := s.cloneStateLocked()
				s.mu.Unlock()
				s.broadcastState(st)
				return
			}
		}
		s.mu.Unlock()
	}()

	select {
	case <-readyCh:
		s.mu.Lock()
		if s.activeID == myID {
			s.state.Busy = false
			s.state.Active = true
			s.state.UDID = udid
			s.state.Latitude = lat
			s.state.Longitude = lng
			s.state.Label = label
			s.state.Method = methodName
			s.state.Error = ""
			s.state.Message = msgText
			st := s.cloneStateLocked()
			s.mu.Unlock()
			s.broadcastState(st)
			return nil
		}
		s.mu.Unlock()
		return nil

	case waitErr := <-errCh:
		outBuf.Lock()
		detail := strings.Join(lastLines, " | ")
		outBuf.Unlock()
		if detail != "" {
			return errors.New(detail)
		}
		if waitErr != nil {
			return waitErr
		}
		return errors.New(locales.ErrExitedEarly())

	case <-time.After(20 * time.Second):
		s.stopActiveProcess()
		return errors.New(locales.ErrConnectTimeout())
	}
}

func (s *SpooferService) runPymobiledevice3Clear(ctx context.Context, targetID string, useRemotePairing bool) error {
	bin, prefixArgs, err := findPymobiledevice3()
	if err != nil {
		return err
	}
	cctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	modeFlag := "--userspace"
	if useRemotePairing {
		modeFlag = "--remotepairing"
	}
	args := append(prefixArgs, "developer", "dvt", "simulate-location", "clear", modeFlag, "--udid", targetID)
	cmd := exec.CommandContext(cctx, bin, args...)
	cmd.Env = augmentedEnv()
	out, err := cmd.CombinedOutput()
	if err != nil {
		msg := strings.TrimSpace(string(out))
		if msg != "" {
			return errors.New(msg)
		}
		return err
	}
	return nil
}

// --- iOS 27+ Wi-Fi Remote Pairing (pair-host) ---

var pairPinRe = regexp.MustCompile(`Enter this code on your device:\s*(\d{6})`)

func parsePairPin(line string) (string, bool) {
	m := pairPinRe.FindStringSubmatch(line)
	if len(m) == 2 {
		return m[1], true
	}
	return "", false
}

// GetPairingState returns the current Wi-Fi pairing session state.
func (s *SpooferService) GetPairingState() PairingState {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pairState
}

// StopWifiPairing stops any active remote pair-host session.
func (s *SpooferService) StopWifiPairing() PairingState {
	s.mu.Lock()
	cmd := s.pairCmd
	done := s.pairDone
	s.pairCmd = nil
	s.pairDone = nil
	s.pairID++
	s.pairState = PairingState{Status: "idle"}
	ps := s.pairState
	s.mu.Unlock()

	if cmd != nil && cmd.Process != nil {
		_ = cmd.Process.Signal(syscall.SIGINT)
		if done != nil {
			select {
			case <-done:
			case <-time.After(1500 * time.Millisecond):
				_ = cmd.Process.Kill()
				select {
				case <-done:
				case <-time.After(1 * time.Second):
				}
			}
		} else {
			_ = cmd.Process.Kill()
		}
	}
	_ = PairingEvent.Broadcast(ps)
	return ps
}

// StartWifiPairing starts `pymobiledevice3 remote pair-host` so an iOS 27+
// device can pair over Wi-Fi from Settings > Developer > Paired Macs.
func (s *SpooferService) StartWifiPairing(hostName string) (PairingState, error) {
	s.StopWifiPairing()

	bin, prefixArgs, err := findPymobiledevice3()
	if err != nil {
		return PairingState{Status: "error", Error: err.Error()}, err
	}
	hostName = strings.TrimSpace(hostName)
	if hostName == "" {
		hostName = "MacBook-Location"
	}
	args := append(prefixArgs, "remote", "pair-host", "--name", hostName, "--timeout", "300")
	cmd := exec.Command(bin, args...)
	cmd.Env = augmentedEnv()

	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return PairingState{Status: "error", Error: err.Error()}, err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return PairingState{Status: "error", Error: err.Error()}, err
	}
	if err := cmd.Start(); err != nil {
		return PairingState{Status: "error", Error: err.Error()}, err
	}

	pairDone := make(chan struct{})
	s.mu.Lock()
	s.pairCmd = cmd
	s.pairDone = pairDone
	s.pairID++
	myID := s.pairID
	s.pairState = PairingState{
		Active:   true,
		HostName: hostName,
		Status:   "waiting",
		Message:  locales.PairWaiting(hostName),
	}
	ps := s.pairState
	s.appendLogLocked(locales.LogPairStarted(hostName))
	st := s.cloneStateLocked()
	s.mu.Unlock()
	_ = PairingEvent.Broadcast(ps)
	s.broadcastState(st)

	readPipe := func(r io.Reader) {
		sc := bufio.NewScanner(r)
		for sc.Scan() {
			line := strings.TrimSpace(sc.Text())
			if line == "" {
				continue
			}
			s.mu.Lock()
			if s.pairID != myID {
				s.mu.Unlock()
				return
			}
			s.appendLogLocked("[pair] " + line)
			changed := false
			if pin, ok := parsePairPin(line); ok {
				s.pairState.PIN = pin
				s.pairState.Status = "pin"
				s.pairState.Message = locales.PairEnterPin()
				changed = true
			} else if strings.HasPrefix(line, "name:") {
				s.pairState.DeviceName = strings.TrimSpace(strings.TrimPrefix(line, "name:"))
				changed = true
			} else if strings.HasPrefix(line, "udid:") {
				s.pairState.DeviceUDID = strings.TrimSpace(strings.TrimPrefix(line, "udid:"))
				changed = true
			} else if strings.Contains(line, "Pairing record written to") {
				s.pairState.Status = "paired"
				s.pairState.Active = false
				s.pairState.Message = locales.PairSuccess()
				changed = true
			}
			curPS := s.pairState
			curST := s.cloneStateLocked()
			s.mu.Unlock()
			if changed {
				_ = PairingEvent.Broadcast(curPS)
			}
			s.broadcastState(curST)
		}
	}

	go readPipe(stdout)
	go readPipe(stderr)

	go func() {
		waitErr := cmd.Wait()
		close(pairDone)
		s.mu.Lock()
		if s.pairID != myID {
			s.mu.Unlock()
			return
		}
		s.pairCmd = nil
		s.pairDone = nil
		s.pairState.Active = false
		if s.pairState.Status != "paired" {
			s.pairState.Status = "error"
			if waitErr != nil {
				s.pairState.Error = locales.ErrPairTimeout()
			} else {
				s.pairState.Error = locales.ErrPairExited()
			}
		}
		curPS := s.pairState
		s.mu.Unlock()
		_ = PairingEvent.Broadcast(curPS)
	}()

	return ps, nil
}

func (s *SpooferService) runDevicectlSet(ctx context.Context, udid string, lat, lng float64) error {
	cctx, cancel := context.WithTimeout(ctx, 12*time.Second)
	defer cancel()
	cmd := exec.CommandContext(cctx, "/usr/bin/xcrun", "devicectl", "device", "simulate", "location", "coordinate",
		"--device", udid,
		fmt.Sprintf("--latitude=%.7f", lat),
		fmt.Sprintf("--longitude=%.7f", lng),
	)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func (s *SpooferService) runDevicectlClear(ctx context.Context, udid string) error {
	cctx, cancel := context.WithTimeout(ctx, 12*time.Second)
	defer cancel()
	cmd := exec.CommandContext(cctx, "/usr/bin/xcrun", "devicectl", "device", "simulate", "location", "clear", "--device", udid)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

// --- Favorites persistence ---

func (s *SpooferService) favoritesPath() string {
	s.mu.Lock()
	dir := s.dataDir
	s.mu.Unlock()
	if dir == "" {
		if cfg, err := os.UserConfigDir(); err == nil {
			dir = filepath.Join(cfg, "iOS Location")
		} else {
			dir = os.TempDir()
		}
	}
	return filepath.Join(dir, "favorites.json")
}

func (s *SpooferService) ListFavorites() ([]Favorite, error) {
	path := s.favoritesPath()
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return []Favorite{}, nil
		}
		return nil, err
	}
	var favs []Favorite
	if err := json.Unmarshal(raw, &favs); err != nil {
		return []Favorite{}, nil
	}
	return favs, nil
}

func (s *SpooferService) saveFavorites(favs []Favorite) error {
	path := s.favoritesPath()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(favs, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o600)
}

func (s *SpooferService) AddFavorite(name string, lat, lng float64) (Favorite, error) {
	if err := validateCoords(lat, lng); err != nil {
		return Favorite{}, err
	}
	name = strings.TrimSpace(name)
	if name == "" {
		name = fmt.Sprintf("%.4f, %.4f", lat, lng)
	}
	if len([]rune(name)) > 40 {
		name = string([]rune(name)[:40])
	}
	favs, err := s.ListFavorites()
	if err != nil {
		return Favorite{}, err
	}
	fav := Favorite{
		ID:        strconv.FormatInt(time.Now().UnixNano(), 36),
		Name:      name,
		Latitude:  lat,
		Longitude: lng,
		CreatedAt: time.Now().UTC().Format(time.RFC3339),
	}
	favs = append([]Favorite{fav}, favs...)
	if len(favs) > 100 {
		favs = favs[:100]
	}
	if err := s.saveFavorites(favs); err != nil {
		return Favorite{}, err
	}
	return fav, nil
}

func (s *SpooferService) DeleteFavorite(id string) ([]Favorite, error) {
	favs, err := s.ListFavorites()
	if err != nil {
		return nil, err
	}
	out := make([]Favorite, 0, len(favs))
	for _, f := range favs {
		if f.ID != id {
			out = append(out, f)
		}
	}
	if err := s.saveFavorites(out); err != nil {
		return nil, err
	}
	return out, nil
}

// --- Geocoding / Search ---

var coordPairRe = regexp.MustCompile(`^\s*(-?\d+(?:\.\d+)?)\s*[,\s]\s*(-?\d+(?:\.\d+)?)\s*$`)

func parseCoordQuery(q string) (float64, float64, bool) {
	m := coordPairRe.FindStringSubmatch(q)
	if len(m) != 3 {
		return 0, 0, false
	}
	lat, err1 := strconv.ParseFloat(m[1], 64)
	lng, err2 := strconv.ParseFloat(m[2], 64)
	if err1 != nil || err2 != nil || validateCoords(lat, lng) != nil {
		return 0, 0, false
	}
	return lat, lng, true
}

// SearchPlace searches OpenStreetMap Nominatim (or parses direct coordinates).
func (s *SpooferService) SearchPlace(ctx context.Context, query string) ([]SearchResult, error) {
	q := strings.TrimSpace(query)
	if q == "" {
		return []SearchResult{}, nil
	}
	if lat, lng, ok := parseCoordQuery(q); ok {
		return []SearchResult{{
			Name:      locales.SearchDirectCoordName(fmt.Sprintf("%.5f", lat), fmt.Sprintf("%.5f", lng)),
			Sub:       locales.SearchDirectCoordSub(),
			Latitude:  lat,
			Longitude: lng,
			Direct:    true,
		}}, nil
	}

	cctx, cancel := context.WithTimeout(ctx, 8*time.Second)
	defer cancel()

	acceptLang := "en,zh-CN"
	if s.GetLocale() == "zh-Hans" {
		acceptLang = "zh-CN,en"
	}
	endpoint := "https://nominatim.openstreetmap.org/search?format=json&limit=6&accept-language=" + acceptLang + "&q=" + url.QueryEscape(q)
	req, err := http.NewRequestWithContext(cctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", "iOS-Location-MyGo/0.1 (local desktop utility)")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	var raw []struct {
		DisplayName string `json:"display_name"`
		Lat         string `json:"lat"`
		Lon         string `json:"lon"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&raw); err != nil {
		return nil, err
	}

	results := make([]SearchResult, 0, len(raw))
	for _, it := range raw {
		lat, err1 := strconv.ParseFloat(it.Lat, 64)
		lng, err2 := strconv.ParseFloat(it.Lon, 64)
		if err1 != nil || err2 != nil {
			continue
		}
		parts := strings.Split(it.DisplayName, ",")
		name := strings.TrimSpace(parts[0])
		sub := ""
		if len(parts) > 1 {
			end := len(parts)
			if end > 4 {
				end = 4
			}
			trimmed := make([]string, 0, end-1)
			for _, p := range parts[1:end] {
				trimmed = append(trimmed, strings.TrimSpace(p))
			}
			sub = strings.Join(trimmed, ", ")
		}
		results = append(results, SearchResult{
			Name:      name,
			Sub:       sub,
			Latitude:  lat,
			Longitude: lng,
		})
	}
	return results, nil
}

// --- Discovery helpers ---

type usbmuxEntry struct {
	Identifier     string `json:"Identifier"`
	UniqueDeviceID string `json:"UniqueDeviceID"`
	DeviceName     string `json:"DeviceName"`
	ProductType    string `json:"ProductType"`
	ProductVersion string `json:"ProductVersion"`
	ConnectionType string `json:"ConnectionType"` // "USB" or "Network"
}

type devicectlEntry struct {
	UDID      string
	Name      string
	Model     string
	OSVersion string
	Connected bool
	Transport string
}

func (s *SpooferService) queryUsbmux(ctx context.Context) ([]usbmuxEntry, error) {
	bin, prefixArgs, err := findPymobiledevice3()
	if err != nil {
		return nil, err
	}
	cctx, cancel := context.WithTimeout(ctx, 12*time.Second)
	defer cancel()
	args := append(prefixArgs, "usbmux", "list")
	cmd := exec.CommandContext(cctx, bin, args...)
	cmd.Env = augmentedEnv()
	out, err := cmd.Output()
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok && len(ee.Stderr) > 0 {
			return nil, fmt.Errorf("%v: %s", err, strings.TrimSpace(string(ee.Stderr)))
		}
		return nil, err
	}
	// Strip any leading non-JSON text (e.g. uvx/log prefix).
	idx := strings.IndexByte(string(out), '[')
	if idx > 0 {
		out = out[idx:]
	}
	var entries []usbmuxEntry
	if err := json.Unmarshal(out, &entries); err != nil {
		return nil, errors.New(locales.ErrParseUsbmux(err.Error()))
	}
	return entries, nil
}

func (s *SpooferService) queryDevicectl(ctx context.Context) ([]devicectlEntry, error) {
	cctx, cancel := context.WithTimeout(ctx, 6*time.Second)
	defer cancel()
	cmd := exec.CommandContext(cctx, "/usr/bin/xcrun", "devicectl", "list", "devices", "--json-output", "-")
	out, err := cmd.Output()
	if err != nil {
		return nil, err
	}
	var payload struct {
		Result struct {
			Devices []struct {
				ConnectionProperties struct {
					TransportType string `json:"transportType"`
					TunnelState   string `json:"tunnelState"`
					PairingState  string `json:"pairingState"`
				} `json:"connectionProperties"`
				DeviceProperties struct {
					Name            string `json:"name"`
					OSVersionNumber string `json:"osVersionNumber"`
				} `json:"deviceProperties"`
				HardwareProperties struct {
					UDID          string `json:"udid"`
					MarketingName string `json:"marketingName"`
					Reality       string `json:"reality"`
				} `json:"hardwareProperties"`
			} `json:"devices"`
		} `json:"result"`
	}
	if err := json.Unmarshal(out, &payload); err != nil {
		return nil, err
	}
	var res []devicectlEntry
	for _, d := range payload.Result.Devices {
		if d.HardwareProperties.Reality != "physical" || d.HardwareProperties.UDID == "" {
			continue
		}
		connected := d.ConnectionProperties.TunnelState == "connected" ||
			d.ConnectionProperties.TransportType == "wired" ||
			d.ConnectionProperties.TransportType == "localNetwork"
		res = append(res, devicectlEntry{
			UDID:      d.HardwareProperties.UDID,
			Name:      d.DeviceProperties.Name,
			Model:     d.HardwareProperties.MarketingName,
			OSVersion: d.DeviceProperties.OSVersionNumber,
			Connected: connected,
			Transport: d.ConnectionProperties.TransportType,
		})
	}
	return res, nil
}

type remoteWifiEntry struct {
	Address    string `json:"address"`
	Port       int    `json:"port"`
	Identifier string `json:"identifier"`
}

func (s *SpooferService) queryRemoteBrowse(ctx context.Context) ([]remoteWifiEntry, error) {
	bin, prefixArgs, err := findPymobiledevice3()
	if err != nil {
		return nil, err
	}
	cctx, cancel := context.WithTimeout(ctx, 8*time.Second)
	defer cancel()
	args := append(prefixArgs, "remote", "browse", "--no-native", "--timeout", "2")
	cmd := exec.CommandContext(cctx, bin, args...)
	cmd.Env = augmentedEnv()
	out, err := cmd.Output()
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok && len(ee.Stderr) > 0 {
			return nil, fmt.Errorf("%v: %s", err, strings.TrimSpace(string(ee.Stderr)))
		}
		return nil, err
	}
	idx := strings.IndexByte(string(out), '{')
	if idx > 0 {
		out = out[idx:]
	}
	var payload struct {
		Wifi []remoteWifiEntry `json:"wifi"`
	}
	if err := json.Unmarshal(out, &payload); err != nil {
		return nil, errors.New(locales.ErrParseRemoteBrowse(err.Error()))
	}
	return payload.Wifi, nil
}

func mergeDevices(usbmux []usbmuxEntry, devctl []devicectlEntry, remoteWifi []remoteWifiEntry) []Device {
	byUDID := make(map[string]*Device)
	hasUSB := make(map[string]bool)
	hasWiFi := make(map[string]bool)

	for _, u := range usbmux {
		udid := u.UniqueDeviceID
		if udid == "" {
			udid = u.Identifier
		}
		if udid == "" {
			continue
		}
		d, exists := byUDID[udid]
		if !exists {
			d = &Device{
				UDID:      udid,
				Name:      u.DeviceName,
				Model:     u.ProductType,
				OSVersion: u.ProductVersion,
			}
			byUDID[udid] = d
		}
		d.CanUserspace = true
		if strings.EqualFold(u.ConnectionType, "Network") {
			hasWiFi[udid] = true
		} else {
			hasUSB[udid] = true
		}
	}

	// Merge iOS 17+/27+ Wi-Fi RemotePairing devices (work even when usbmuxd is empty).
	for _, rw := range remoteWifi {
		if rw.Identifier == "" {
			continue
		}
		d, exists := byUDID[rw.Identifier]
		if !exists {
			d = &Device{
				UDID: rw.Identifier,
			}
			byUDID[rw.Identifier] = d
		}
		d.CanRemotePairing = true
		d.RemoteIdentifier = rw.Identifier
		hasWiFi[rw.Identifier] = true
	}

	for _, dc := range devctl {
		d, exists := byUDID[dc.UDID]
		if !exists {
			if !dc.Connected {
				continue
			}
			d = &Device{
				UDID:      dc.UDID,
				Name:      dc.Name,
				Model:     dc.Model,
				OSVersion: dc.OSVersion,
			}
			byUDID[dc.UDID] = d
		} else {
			if dc.Name != "" && d.Name == "" {
				d.Name = dc.Name
			}
			if dc.Model != "" {
				d.Model = dc.Model
			}
			if dc.OSVersion != "" {
				d.OSVersion = dc.OSVersion
			}
		}
		if dc.Connected {
			d.CanDevicectl = true
			if dc.Transport == "wired" {
				hasUSB[dc.UDID] = true
			} else if dc.Transport == "localNetwork" {
				hasWiFi[dc.UDID] = true
			}
		}
	}

	out := make([]Device, 0, len(byUDID))
	for udid, d := range byUDID {
		switch {
		case hasUSB[udid] && hasWiFi[udid]:
			d.Transport = "usb+wifi"
		case hasWiFi[udid]:
			d.Transport = "wifi"
		default:
			d.Transport = "usb"
		}
		if d.Name == "" {
			d.Name = udid
		}
		out = append(out, *d)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

func validateCoords(lat, lng float64) error {
	if math.IsNaN(lat) || math.IsInf(lat, 0) || lat < -90 || lat > 90 {
		return errors.New(locales.ErrLatOutOfRange(fmt.Sprintf("%v", lat)))
	}
	if math.IsNaN(lng) || math.IsInf(lng, 0) || lng < -180 || lng > 180 {
		return errors.New(locales.ErrLngOutOfRange(fmt.Sprintf("%v", lng)))
	}
	return nil
}

func augmentedEnv() []string {
	home, _ := os.UserHomeDir()
	extra := []string{
		filepath.Join(home, ".local", "bin"),
		"/opt/homebrew/bin",
		"/usr/local/bin",
		"/usr/bin",
		"/bin",
	}
	env := append([]string(nil), os.Environ()...)
	hasPath := false
	for i, kv := range env {
		if strings.HasPrefix(kv, "PATH=") {
			env[i] = "PATH=" + strings.Join(extra, ":") + ":" + strings.TrimPrefix(kv, "PATH=")
			hasPath = true
		}
	}
	if !hasPath {
		env = append(env, "PATH="+strings.Join(extra, ":"))
	}
	env = append(env, "NO_COLOR=1", "TERM=dumb", "PYTHONUNBUFFERED=1")
	return env
}

func findPymobiledevice3() (string, []string, error) {
	home, _ := os.UserHomeDir()
	candidates := []string{
		filepath.Join(home, ".local", "bin", "pymobiledevice3"),
		"/opt/homebrew/bin/pymobiledevice3",
		"/usr/local/bin/pymobiledevice3",
	}
	for _, p := range candidates {
		if info, err := os.Stat(p); err == nil && !info.IsDir() {
			return p, nil, nil
		}
	}
	if p, err := exec.LookPath("pymobiledevice3"); err == nil {
		return p, nil, nil
	}
	uvxCandidates := []string{
		filepath.Join(home, ".local", "bin", "uvx"),
		"/opt/homebrew/bin/uvx",
	}
	for _, p := range uvxCandidates {
		if info, err := os.Stat(p); err == nil && !info.IsDir() {
			return p, []string{"pymobiledevice3"}, nil
		}
	}
	if p, err := exec.LookPath("uvx"); err == nil {
		return p, []string{"pymobiledevice3"}, nil
	}
	return "", nil, errors.New(locales.ErrPymobiledeviceMissing())
}
