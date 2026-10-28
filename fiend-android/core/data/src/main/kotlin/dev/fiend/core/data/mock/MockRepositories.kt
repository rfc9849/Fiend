package dev.fiend.core.data.mock

import dev.fiend.core.data.AppsRepository
import dev.fiend.core.data.LogRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLine
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.update

@Singleton
class MockSettingsRepository @Inject constructor() : SettingsRepository {
    private val state = MutableStateFlow(
        AppSettings(splitTunnelApps = setOf("com.example.bank", "com.netflix.mediaclient")),
    )

    override val settings: StateFlow<AppSettings> = state.asStateFlow()

    override suspend fun update(transform: (AppSettings) -> AppSettings) {
        state.update(transform)
    }
}

@Singleton
class MockAppsRepository @Inject constructor() : AppsRepository {
    override val apps: Flow<List<InstalledApp>> = flowOf(MockData.apps.sortedBy { it.label })
}

@Singleton
class MockLogRepository @Inject constructor() : LogRepository {
    private val state = MutableStateFlow(MockData.bootLog(System.currentTimeMillis()))

    override val lines: StateFlow<List<LogLine>> = state.asStateFlow()

    override fun append(line: LogLine) {
        state.update { (it + line).takeLast(MAX_LINES) }
    }

    override fun clear() {
        state.value = emptyList()
    }

    private companion object {
        const val MAX_LINES = 500
    }
}
