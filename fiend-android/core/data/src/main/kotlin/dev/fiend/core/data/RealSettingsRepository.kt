package dev.fiend.core.data

import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import dev.fiend.core.data.storage.PreferencesStore
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.ThemeMode
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.LogLevel
import dev.fiend.core.model.LogLine
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

@Singleton
internal class RealSettingsRepository @Inject constructor(
    private val store: PreferencesStore,
    logs: LogRepository,
    @ApplicationScope scope: CoroutineScope,
) : SettingsRepository {
    private val mutableSettings = MutableStateFlow(AppSettings())
    override val settings = mutableSettings.asStateFlow()
    private val ready = CompletableDeferred<Unit>()
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
        store.data.edit { values ->
            val next = transform(decode(values))
            values[THEME] = next.themeMode.name
            values[DYNAMIC] = next.dynamicColor
            values[CONNECT] = next.connectOnStart
            values[LAN] = next.bypassLan
            values[IPV6] = next.ipv6
            values[REMOTE_DNS] = true
            values[SPLIT_MODE] = next.splitTunnelMode.name
            values[SPLIT_APPS] = next.splitTunnelApps
        }
    }

    private fun decode(values: Preferences) = AppSettings(
        themeMode = ThemeMode.entries.firstOrNull { it.name == values[THEME] } ?: ThemeMode.System,
        dynamicColor = values[DYNAMIC] ?: true,
        connectOnStart = values[CONNECT] ?: false,
        bypassLan = values[LAN] ?: true,
        ipv6 = values[IPV6] ?: true,
        remoteDns = true,
        splitTunnelMode = SplitTunnelMode.entries.firstOrNull { it.name == values[SPLIT_MODE] } ?: SplitTunnelMode.AllApps,
        splitTunnelApps = values[SPLIT_APPS] ?: emptySet(),
    )

    private companion object {
        val THEME = stringPreferencesKey("theme_mode")
        val DYNAMIC = booleanPreferencesKey("dynamic_color")
        val CONNECT = booleanPreferencesKey("connect_on_start")
        val LAN = booleanPreferencesKey("bypass_lan")
        val IPV6 = booleanPreferencesKey("ipv6")
        val REMOTE_DNS = booleanPreferencesKey("remote_dns")
        val SPLIT_MODE = stringPreferencesKey("split_mode")
        val SPLIT_APPS = stringSetPreferencesKey("split_apps")
    }
}
