package dev.fiend.core.data.mock

import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.Profile

object MockData {
    val templates = listOf(
        FingerprintTemplate("chrome-android", "Chrome 141", "Android"),
        FingerprintTemplate("chrome-windows", "Chrome 141", "Windows"),
        FingerprintTemplate("firefox-android", "Firefox 144", "Android"),
        FingerprintTemplate("safari-ios", "Safari 26", "iOS"),
    )

    private const val HOUR = 3_600_000L

    fun profiles(now: Long) = listOf(
        Profile(
            id = "ams",
            name = "Amsterdam",
            host = "ams.edge.example.net",
            port = 443,
            secret = "8c1f0e7a52d94b6f3a0e9d21c7b45f88e0a3d6c19b2f7e4a5d8c0b1e6f2a9f3a",
            templateId = "chrome-android",
            countryCode = "NL",
            latencyMs = 38,
            lastConnectedAt = now - 2 * HOUR,
        ),
        Profile(
            id = "fra",
            name = "Frankfurt",
            host = "fra-2.example.org",
            port = 443,
            secret = "1b7e3c9d0f2a4e6b8c1d3f5a7e9b0c2d4f6a8e1b3c5d7f9a0b2c4e6f8a1b7e20",
            templateId = "chrome-android",
            countryCode = "DE",
            latencyMs = 44,
            lastConnectedAt = now - 26 * HOUR,
        ),
        Profile(
            id = "hel",
            name = "Helsinki",
            host = "hel.example.com",
            port = 8443,
            secret = "f0e1d2c3b4a5968778695a4b3c2d1e0ff0e1d2c3b4a5968778695a4b3c2d1e0c",
            templateId = "firefox-android",
            countryCode = "FI",
            latencyMs = 61,
        ),
        Profile(
            id = "home",
            name = "Home server",
            host = "vpn.home.arpa",
            port = 443,
            secret = "a1a2a3a4a5a6a7a8a9b0b1b2b3b4b5b6b7b8b9c0c1c2c3c4c5c6c7c8c9d0d1d2",
            templateId = "chrome-windows",
            latencyMs = 12,
        ),
        Profile(
            id = "tyo",
            name = "Tokyo",
            host = "tyo.example.jp",
            port = 443,
            secret = "5e4d3c2b1a0f9e8d7c6b5a4f3e2d1c0b5e4d3c2b1a0f9e8d7c6b5a4f3e2d7b41",
            templateId = "safari-ios",
            countryCode = "JP",
            latencyMs = 212,
        ),
    )

    val apps = listOf(
        InstalledApp("com.android.chrome", "Chrome"),
        InstalledApp("com.google.android.youtube", "YouTube"),
        InstalledApp("com.google.android.gm", "Gmail"),
        InstalledApp("com.google.android.apps.maps", "Maps"),
        InstalledApp("org.telegram.messenger", "Telegram"),
        InstalledApp("com.whatsapp", "WhatsApp"),
        InstalledApp("com.spotify.music", "Spotify"),
        InstalledApp("com.netflix.mediaclient", "Netflix"),
        InstalledApp("com.instagram.android", "Instagram"),
        InstalledApp("com.discord", "Discord"),
        InstalledApp("org.mozilla.firefox", "Firefox"),
        InstalledApp("com.example.bank", "My Bank"),
        InstalledApp("com.valvesoftware.android.steam.community", "Steam"),
        InstalledApp("com.google.android.apps.photos", "Photos", isSystem = true),
        InstalledApp("com.android.vending", "Play Store", isSystem = true),
    )

    fun bootLog(now: Long) = listOf(
        LogLine(now - 9_000, LogLevel.Info, "fiend_mobile", "runtime started, 4 workers"),
        LogLine(now - 8_800, LogLevel.Debug, "fiend_tun", "fake-ip pool 198.18.0.0/15 ready"),
        LogLine(now - 8_700, LogLevel.Info, "fiend_core::config", "loaded 5 profiles"),
    )
}
