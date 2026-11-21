use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    #[serde(rename = "zh-Hans")]
    ZhHans,
    #[serde(rename = "en")]
    En,
}

impl Locale {
    pub fn normalize(tag: &str) -> Self {
        if tag.trim().to_ascii_lowercase().starts_with("zh") {
            Self::ZhHans
        } else {
            Self::En
        }
    }

    pub fn detect_system() -> Self {
        if let Ok(out) = Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let cleaned = line.trim().trim_matches(|c| c == '"' || c == ',' || c == '(' || c == ')');
                if !cleaned.is_empty() {
                    return Self::normalize(cleaned);
                }
            }
        }
        if let Ok(lang) = std::env::var("LANG") {
            return Self::normalize(&lang);
        }
        Self::ZhHans
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::ZhHans => Self::En,
            Self::En => Self::ZhHans,
        }
    }

    pub fn google_hl(self) -> &'static str {
        match self {
            Self::ZhHans => "zh-CN",
            Self::En => "en",
        }
    }

    pub fn accept_language(self) -> &'static str {
        match self {
            Self::ZhHans => "zh-CN,en",
            Self::En => "en,zh-CN",
        }
    }

    pub fn lang_btn(self) -> &'static str {
        match self {
            Self::ZhHans => "EN",
            Self::En => "中",
        }
    }

    pub fn app_title(self) -> &'static str {
        "iOS Location"
    }

    pub fn menu_file(self) -> &'static str {
        match self {
            Self::ZhHans => "文件",
            Self::En => "File",
        }
    }

    pub fn menu_edit(self) -> &'static str {
        match self {
            Self::ZhHans => "编辑",
            Self::En => "Edit",
        }
    }

    pub fn menu_view(self) -> &'static str {
        match self {
            Self::ZhHans => "显示",
            Self::En => "View",
        }
    }

    pub fn menu_quit(self) -> &'static str {
        match self {
            Self::ZhHans => "退出 iOS Location",
            Self::En => "Quit iOS Location",
        }
    }

    pub fn menu_toggle_panel(self) -> &'static str {
        match self {
            Self::ZhHans => "显示 / 隐藏控制面板",
            Self::En => "Toggle Control Panel",
        }
    }

    pub fn menu_focus_search(self) -> &'static str {
        match self {
            Self::ZhHans => "搜索地点",
            Self::En => "Search Places",
        }
    }

    pub fn menu_switch_lang(self) -> &'static str {
        match self {
            Self::ZhHans => "Switch to English",
            Self::En => "切换到简体中文",
        }
    }

    pub fn menu_toggle_theme(self) -> &'static str {
        match self {
            Self::ZhHans => "切换浅色 / 深色外观",
            Self::En => "Toggle Light / Dark Mode",
        }
    }

    pub fn expand_label(self) -> &'static str {
        match self {
            Self::ZhHans => "控制面板",
            Self::En => "Controls",
        }
    }

    pub fn offline(self) -> &'static str {
        match self {
            Self::ZhHans => "离线",
            Self::En => "Offline",
        }
    }

    pub fn scanning_devices(self) -> &'static str {
        match self {
            Self::ZhHans => "正在扫描设备...",
            Self::En => "Scanning devices...",
        }
    }

    pub fn no_device(self) -> &'static str {
        match self {
            Self::ZhHans => "未检测到已连接设备",
            Self::En => "No connected device found",
        }
    }

    pub fn search_placeholder(self) -> &'static str {
        match self {
            Self::ZhHans => "搜索地点或粘贴经纬度",
            Self::En => "Search place or paste lat, lng",
        }
    }

    pub fn searching(self) -> &'static str {
        match self {
            Self::ZhHans => "搜索中...",
            Self::En => "Searching...",
        }
    }

    pub fn search_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "未找到匹配地点",
            Self::En => "No matching places found",
        }
    }

    pub fn search_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "搜索失败: ",
            Self::En => "Search failed: ",
        }
    }

    pub fn jumped_coord(self) -> &'static str {
        match self {
            Self::ZhHans => "已跳转到目标坐标",
            Self::En => "Jumped to coordinates",
        }
    }

    pub fn status_idle(self) -> &'static str {
        match self {
            Self::ZhHans => "未模拟定位",
            Self::En => "Not Simulating",
        }
    }

    pub fn status_syncing(self) -> &'static str {
        match self {
            Self::ZhHans => "同步中...",
            Self::En => "Syncing...",
        }
    }

    pub fn status_syncing_detail(self) -> &'static str {
        match self {
            Self::ZhHans => "正在下发定位到设备...",
            Self::En => "Syncing location to device...",
        }
    }

    pub fn status_locked(self) -> &'static str {
        match self {
            Self::ZhHans => "已锁定模拟定位",
            Self::En => "Location Locked",
        }
    }

    pub fn status_locked_prefix(self) -> &'static str {
        match self {
            Self::ZhHans => "已锁定 · ",
            Self::En => "Locked · ",
        }
    }

    pub fn status_hint(self) -> &'static str {
        match self {
            Self::ZhHans => "拖动地图选点，然后点击「锁定定位」",
            Self::En => "Drag the map, then click \"Lock Location\"",
        }
    }

    pub fn status_active_devicectl(self) -> &'static str {
        match self {
            Self::ZhHans => "已通过 Xcode devicectl 生效",
            Self::En => "Active via Xcode devicectl",
        }
    }

    pub fn status_active_remote(self) -> &'static str {
        match self {
            Self::ZhHans => "已通过独立 RemotePairing 无线隧道生效",
            Self::En => "Active via standalone RemotePairing Wi-Fi tunnel",
        }
    }

    pub fn status_active_userspace(self) -> &'static str {
        match self {
            Self::ZhHans => "已通过 Wi-Fi 用户态隧道生效",
            Self::En => "Active via Wi-Fi userspace tunnel",
        }
    }

    pub fn status_set_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "定位设置失败",
            Self::En => "Failed to set location",
        }
    }

    pub fn status_clearing(self) -> &'static str {
        match self {
            Self::ZhHans => "正在恢复真实定位...",
            Self::En => "Restoring real GPS location...",
        }
    }

    pub fn status_cleared(self) -> &'static str {
        match self {
            Self::ZhHans => "已恢复真实定位",
            Self::En => "Real GPS location restored",
        }
    }

    pub fn status_disconnected(self) -> &'static str {
        match self {
            Self::ZhHans => "无线定位会话已断开",
            Self::En => "Wireless location session disconnected",
        }
    }

    pub fn method_xcode(self) -> &'static str {
        "Xcode"
    }

    pub fn method_remote(self) -> &'static str {
        match self {
            Self::ZhHans => "Wi-Fi 直连",
            Self::En => "Wi-Fi Direct",
        }
    }

    pub fn method_wifi(self) -> &'static str {
        "Wi-Fi"
    }

    pub fn copied_toast(self) -> &'static str {
        match self {
            Self::ZhHans => "已复制坐标: ",
            Self::En => "Copied: ",
        }
    }

    pub fn btn_lock(self) -> &'static str {
        match self {
            Self::ZhHans => "锁定定位",
            Self::En => "Lock Location",
        }
    }

    pub fn btn_clear(self) -> &'static str {
        match self {
            Self::ZhHans => "恢复真实",
            Self::En => "Reset GPS",
        }
    }

    pub fn tab_cities(self) -> &'static str {
        match self {
            Self::ZhHans => "快捷城市",
            Self::En => "Quick Cities",
        }
    }

    pub fn tab_fav(self) -> &'static str {
        match self {
            Self::ZhHans => "我的收藏",
            Self::En => "Favorites",
        }
    }

    pub fn btn_add_fav(self) -> &'static str {
        match self {
            Self::ZhHans => "+ 收藏",
            Self::En => "+ Save",
        }
    }

    pub fn fav_placeholder(self) -> &'static str {
        match self {
            Self::ZhHans => "输入收藏名称（如：洛杉矶办公室）",
            Self::En => "Bookmark name (e.g. LA Office)",
        }
    }

    pub fn btn_save(self) -> &'static str {
        match self {
            Self::ZhHans => "保存",
            Self::En => "Save",
        }
    }

    pub fn btn_cancel(self) -> &'static str {
        match self {
            Self::ZhHans => "取消",
            Self::En => "Cancel",
        }
    }

    pub fn fav_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "暂无收藏地点，移动准星后点击右上角「+ 收藏」保存",
            Self::En => "No saved places yet. Move the reticle and click \"+ Save\".",
        }
    }

    pub fn fav_teleport(self) -> &'static str {
        match self {
            Self::ZhHans => "传送",
            Self::En => "Go",
        }
    }

    pub fn fav_added(self) -> &'static str {
        match self {
            Self::ZhHans => "已加入收藏",
            Self::En => "Saved to Favorites",
        }
    }

    pub fn fav_deleted(self) -> &'static str {
        match self {
            Self::ZhHans => "已删除收藏",
            Self::En => "Removed favorite",
        }
    }

    pub fn logs_title(self) -> &'static str {
        match self {
            Self::ZhHans => "运行日志",
            Self::En => "Console",
        }
    }

    pub fn logs_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "等待操作...",
            Self::En => "Waiting for action...",
        }
    }

    pub fn recenter_ok(self) -> &'static str {
        match self {
            Self::ZhHans => "已回到当前锁定位置",
            Self::En => "Centered on locked location",
        }
    }

    pub fn recenter_none(self) -> &'static str {
        match self {
            Self::ZhHans => "当前尚未锁定模拟位置",
            Self::En => "No simulated location locked yet",
        }
    }

    pub fn toast_need_device(self) -> &'static str {
        match self {
            Self::ZhHans => "请先连接 iPhone 并刷新设备列表",
            Self::En => "Connect an iPhone and refresh devices first",
        }
    }

    pub fn toast_locked(self) -> &'static str {
        match self {
            Self::ZhHans => "定位已生效",
            Self::En => "Location locked",
        }
    }

    pub fn toast_set_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "设置失败",
            Self::En => "Failed to set location",
        }
    }

    pub fn toast_cleared(self) -> &'static str {
        match self {
            Self::ZhHans => "已恢复真实定位",
            Self::En => "Real GPS restored",
        }
    }

    pub fn toast_clear_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "恢复失败",
            Self::En => "Failed to restore GPS",
        }
    }

    pub fn toast_paired_prefix(self) -> &'static str {
        match self {
            Self::ZhHans => "已成功无线配对 ",
            Self::En => "Paired wirelessly with ",
        }
    }

    pub fn pair_title(self) -> &'static str {
        match self {
            Self::ZhHans => "iOS 27+ 无线直连配对",
            Self::En => "iOS 27+ Wireless Pairing",
        }
    }

    pub fn pair_subtitle(self) -> &'static str {
        match self {
            Self::ZhHans => "独立 RemotePairing 隧道 · 摆脱 usbmuxd 掉线",
            Self::En => "Standalone RemotePairing tunnel · no usbmuxd drops",
        }
    }

    pub fn pair_step1(self) -> &'static str {
        match self {
            Self::ZhHans => "1. 确保 iPhone 与 Mac 连在同一 Wi-Fi",
            Self::En => "1. Keep iPhone and Mac on the same Wi-Fi network",
        }
    }

    pub fn pair_step2(self) -> &'static str {
        match self {
            Self::ZhHans => "2. 打开 iPhone 设置 → 开发者 → 已配对的 Mac",
            Self::En => "2. Open iPhone Settings → Developer → Paired Macs",
        }
    }

    pub fn pair_step3(self, host: &str) -> String {
        match self {
            Self::ZhHans => format!("3. 在「其他设备」中点击 {host} 并点「配对」"),
            Self::En => format!("3. Under Other Devices tap {host} and choose Pair"),
        }
    }

    pub fn pair_pin_label(self) -> &'static str {
        match self {
            Self::ZhHans => "请在 iPhone 上输入配对码",
            Self::En => "Enter this code on your iPhone",
        }
    }

    pub fn pair_waiting(self, host: &str) -> String {
        match self {
            Self::ZhHans => format!("正在局域网广播「{host}」，等待 iPhone 发起配对..."),
            Self::En => format!("Advertising \"{host}\" on LAN, waiting for iPhone to pair..."),
        }
    }

    pub fn pair_enter_pin(self) -> &'static str {
        match self {
            Self::ZhHans => "请在 iPhone 上输入下方 6 位配对码",
            Self::En => "Enter the 6-digit code below on your iPhone",
        }
    }

    pub fn pair_success(self) -> &'static str {
        match self {
            Self::ZhHans => "无线配对成功！正在刷新设备列表...",
            Self::En => "Wireless pairing succeeded! Refreshing devices...",
        }
    }

    pub fn pair_timeout(self) -> &'static str {
        match self {
            Self::ZhHans => "配对已结束或超时，请重试",
            Self::En => "Pairing ended or timed out, please retry",
        }
    }

    pub fn pair_restart(self) -> &'static str {
        match self {
            Self::ZhHans => "重新广播",
            Self::En => "Re-advertise",
        }
    }

    pub fn err_no_device(self) -> &'static str {
        match self {
            Self::ZhHans => "未检测到已连接的 iPhone（请确认同一 Wi-Fi 或 USB 已连接）",
            Self::En => "No connected iPhone detected (ensure same Wi-Fi or USB cable)",
        }
    }

    pub fn err_pymobiledevice_missing(self) -> &'static str {
        match self {
            Self::ZhHans => "未找到 pymobiledevice3（请运行 uv tool install pymobiledevice3）",
            Self::En => "pymobiledevice3 not found (run `uv tool install pymobiledevice3`)",
        }
    }

    pub fn err_connect_timeout(self) -> &'static str {
        match self {
            Self::ZhHans => "连接设备超时（请确认 iPhone 与 Mac 在同一 Wi-Fi 且屏幕已解锁）",
            Self::En => "Connection timed out (ensure iPhone and Mac share Wi-Fi and iPhone is unlocked)",
        }
    }

    pub fn search_direct_sub(self) -> &'static str {
        match self {
            Self::ZhHans => "WGS-84 经纬度直跳",
            Self::En => "Direct WGS-84 jump",
        }
    }

    pub fn cities(self) -> [&'static str; 6] {
        match self {
            Self::ZhHans => ["洛杉矶", "旧金山", "东京塔", "新加坡", "伦敦", "北京"],
            Self::En => ["Los Angeles", "San Francisco", "Tokyo Tower", "Singapore", "London", "Beijing"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locale_normalize() {
        assert_eq!(Locale::normalize("zh"), Locale::ZhHans);
        assert_eq!(Locale::normalize("zh-CN"), Locale::ZhHans);
        assert_eq!(Locale::normalize("zh_Hans_CN"), Locale::ZhHans);
        assert_eq!(Locale::normalize("en-US"), Locale::En);
        assert_eq!(Locale::normalize(""), Locale::En);
        assert_eq!(Locale::En.status_cleared(), "Real GPS location restored");
        assert_eq!(Locale::ZhHans.status_cleared(), "已恢复真实定位");
    }
}
