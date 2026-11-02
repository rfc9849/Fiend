package dev.fiend.core.data

import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import dev.fiend.core.data.storage.PreferencesStore
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.ThemeMode
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.RuleSubscription
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json

@Singleton
internal class RealSettingsRepository @Inject constructor(
    private val store: PreferencesStore,
    logs: LogRepository,
    @ApplicationScope scope: CoroutineScope,
) : SettingsRepository {
    private val mutableSettings = MutableStateFlow(AppSettings())
    override val settings = mutableSettings.asStateFlow()
    private val ready = CompletableDeferred<Unit>()
    private val updateMutex = Mutex()
    init {
        scope.launch {
            try {
                store.data.data.collect { mutableSettings.value = decode(it); ready.complete(Unit) }
            } catch (error: Exception) {
                ready.completeExceptionally(error)
                logs.append(LogLine(System.currentTimeMillis(), LogLevel.Error, "storage", "Cannot read settings"))
            }
        }
    }

    override suspend fun awaitReady() { ready.await() }

    override suspend fun update(transform: (AppSettings) -> AppSettings) {
        awaitReady()
        updateMutex.withLock {
            val saved = store.data.edit { values ->
                val next = transform(decode(values))
                values[THEME] = next.themeMode.name
                values[CONNECT] = next.connectOnStart
                values[LAN] = next.bypassLan
                values[IPV6] = next.ipv6
                values[SPLIT_MODE] = next.splitTunnelMode.name
                values[SPLIT_APPS] = next.splitTunnelApps
                values[DIRECT_RULES] = next.directRules
                values[BLOCK_RULES] = next.blockRules
                values[PROXY_RULES] = next.proxyRules
                values[DPI_RULES] = next.dpiRules
                values[SUBSCRIPTIONS] = encodeSubscriptions(next.subscriptions)
                values[DIRECT_RESOLVER] = next.directResolver
                values[MTU] = next.mtu.coerceIn(AppSettings.MIN_MTU, AppSettings.MAX_MTU)
            }
            val expected = decode(saved)
            settings.first { it == expected }
        }
    }

    private fun decode(values: Preferences) = AppSettings(
        themeMode = ThemeMode.entries.firstOrNull { it.name == values[THEME] } ?: ThemeMode.System,
        connectOnStart = values[CONNECT] ?: false,
        bypassLan = values[LAN] ?: true,
        ipv6 = values[IPV6] ?: true,
        splitTunnelMode = SplitTunnelMode.entries.firstOrNull { it.name == values[SPLIT_MODE] } ?: SplitTunnelMode.AllApps,
        splitTunnelApps = values[SPLIT_APPS] ?: emptySet(),
        directRules = values[DIRECT_RULES] ?: "",
        blockRules = values[BLOCK_RULES] ?: "",
        proxyRules = values[PROXY_RULES] ?: "",
        dpiRules = values[DPI_RULES] ?: "",
        subscriptions = decodeSubscriptions(values[SUBSCRIPTIONS]),
        directResolver = values[DIRECT_RESOLVER] ?: "",
        mtu = (values[MTU] ?: AppSettings.DEFAULT_MTU).coerceIn(AppSettings.MIN_MTU, AppSettings.MAX_MTU),
    )

    private fun encodeSubscriptions(subscriptions: List<RuleSubscription>): String =
        runCatching { json.encodeToString(subscriptions) }.getOrDefault("")

    private fun decodeSubscriptions(value: String?): List<RuleSubscription> {
        if (value.isNullOrBlank()) return emptyList()
        return runCatching { json.decodeFromString<List<RuleSubscription>>(value) }.getOrDefault(emptyList())
    }

    private companion object {
        val json = Json { ignoreUnknownKeys = true }

        val THEME = stringPreferencesKey("theme_mode")
        val CONNECT = booleanPreferencesKey("connect_on_start")
        val LAN = booleanPreferencesKey("bypass_lan")
        val IPV6 = booleanPreferencesKey("ipv6")
        val SPLIT_MODE = stringPreferencesKey("split_mode")
        val SPLIT_APPS = stringSetPreferencesKey("split_apps")
        val DIRECT_RULES = stringPreferencesKey("direct_rules")
        val BLOCK_RULES = stringPreferencesKey("block_rules")
        val PROXY_RULES = stringPreferencesKey("proxy_rules")
        val DPI_RULES = stringPreferencesKey("dpi_rules")
        val SUBSCRIPTIONS = stringPreferencesKey("subscriptions")
        val DIRECT_RESOLVER = stringPreferencesKey("direct_resolver")
        val MTU = intPreferencesKey("mtu")
    }
}
