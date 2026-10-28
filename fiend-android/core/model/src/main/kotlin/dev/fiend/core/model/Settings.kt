package dev.fiend.core.model

enum class ThemeMode { System, Light, Dark }

enum class SplitTunnelMode { AllApps, OnlySelected, ExceptSelected }

data class AppSettings(
    val themeMode: ThemeMode = ThemeMode.System,
    val dynamicColor: Boolean = true,
    val connectOnStart: Boolean = false,
    val bypassLan: Boolean = true,
    val ipv6: Boolean = true,
    val remoteDns: Boolean = true,
    val splitTunnelMode: SplitTunnelMode = SplitTunnelMode.AllApps,
    val splitTunnelApps: Set<String> = emptySet(),
)

data class InstalledApp(
    val packageName: String,
    val label: String,
    val isSystem: Boolean = false,
)

enum class LogLevel { Debug, Info, Warn, Error }

data class LogLine(
    val timeEpochMs: Long,
    val level: LogLevel,
    val target: String,
    val message: String,
)
