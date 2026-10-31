package dev.fiend.core.model

import kotlinx.serialization.Serializable

enum class ThemeMode { System, Light, Dark }

enum class SplitTunnelMode { AllApps, OnlySelected, ExceptSelected }

@Serializable
enum class RuleKind { Direct, Block, Proxy, Dpi }

@Serializable
data class RuleSubscription(
    val id: String,
    val url: String,
    val kind: RuleKind,
    val updatedAtMs: Long = 0,
    val entries: Int = 0,
    val error: String? = null,
    val name: String = "",
) {
    val title: String get() = name.trim().ifEmpty { urlTitle(url) }

    companion object {
        const val MAX_NAME = 64
    }
}

fun urlTitle(url: String): String {
    val path = url.substringBefore('?').substringBefore('#').substringAfter("://")
    val file = path.trimEnd('/').substringAfterLast('/')
    val stem = listOf(".srs", ".txt", ".list", ".json").firstNotNullOfOrNull { suffix ->
        file.takeIf { it.endsWith(suffix) }?.removeSuffix(suffix)
    } ?: file
    return stem.ifEmpty { url }
}

data class RuleFileSpec(val path: String, val kind: RuleKind)

data class AppSettings(
    val themeMode: ThemeMode = ThemeMode.System,
    val dynamicColor: Boolean = true,
    val connectOnStart: Boolean = false,
    val bypassLan: Boolean = true,
    val ipv6: Boolean = true,
    val splitTunnelMode: SplitTunnelMode = SplitTunnelMode.AllApps,
    val splitTunnelApps: Set<String> = emptySet(),
    val directRules: String = "",
    val blockRules: String = "",
    val proxyRules: String = "",
    val dpiRules: String = "",
    val subscriptions: List<RuleSubscription> = emptyList(),
    val directResolver: String = "",
    val mtu: Int = DEFAULT_MTU,
) {
    companion object {
        const val DEFAULT_MTU = 1500
        const val MIN_MTU = 1280
        const val MAX_MTU = 9000
    }
}

data class RuleCounts(
    val domains: Int = 0,
    val exact: Int = 0,
    val keywords: Int = 0,
    val cidrs: Int = 0,
) {
    val total: Int get() = domains + exact + keywords + cidrs
}

data class InstalledApp(
    val packageName: String,
    val label: String,
    val isSystem: Boolean = false,
    val iconPng: ByteArray? = null,
)

enum class LogLevel { Debug, Info, Warn, Error }

data class LogLine(
    val timeEpochMs: Long,
    val level: LogLevel,
    val target: String,
    val message: String,
)
