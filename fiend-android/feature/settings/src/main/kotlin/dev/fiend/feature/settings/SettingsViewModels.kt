package dev.fiend.feature.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import dev.fiend.core.data.AppsRepository
import dev.fiend.core.data.LogRepository
import dev.fiend.core.data.RulesRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.data.SubscriptionRepository
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.InstalledApp
import dev.fiend.core.model.LogLine
import dev.fiend.core.model.RuleCounts
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import dev.fiend.core.model.SplitTunnelMode
import dev.fiend.core.model.ThemeMode
import javax.inject.Inject
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val repository: SettingsRepository,
) : ViewModel() {
    val settings: StateFlow<AppSettings> = repository.settings

    fun update(transform: (AppSettings) -> AppSettings) {
        viewModelScope.launch { repository.update(transform) }
    }

    fun setTheme(mode: ThemeMode) = update { it.copy(themeMode = mode) }
}

data class SplitTunnelUiState(
    val mode: SplitTunnelMode = SplitTunnelMode.AllApps,
    val selected: Set<String> = emptySet(),
    val apps: List<InstalledApp> = emptyList(),
    val query: String = "",
)

@HiltViewModel
class SplitTunnelViewModel @Inject constructor(
    private val settings: SettingsRepository,
    apps: AppsRepository,
) : ViewModel() {
    private val query = MutableStateFlow("")

    val uiState: StateFlow<SplitTunnelUiState> = combine(settings.settings, apps.apps, query) { s, list, q ->
        val needle = q.trim()
        SplitTunnelUiState(
            mode = s.splitTunnelMode,
            selected = s.splitTunnelApps,
            apps = list.filter { needle.isEmpty() || it.label.contains(needle, true) || it.packageName.contains(needle, true) },
            query = q,
        )
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), SplitTunnelUiState())

    fun setMode(mode: SplitTunnelMode) {
        viewModelScope.launch { settings.update { it.copy(splitTunnelMode = mode) } }
    }

    fun toggle(packageName: String) {
        viewModelScope.launch {
            settings.update {
                val next = if (packageName in it.splitTunnelApps) it.splitTunnelApps - packageName else it.splitTunnelApps + packageName
                it.copy(splitTunnelApps = next)
            }
        }
    }

    fun onQueryChange(value: String) {
        query.value = value
    }
}

@HiltViewModel
class LogsViewModel @Inject constructor(
    private val repository: LogRepository,
) : ViewModel() {
    val lines: StateFlow<List<LogLine>> = repository.lines

    fun clear() = repository.clear()
}

data class SubscriptionUi(
    val id: String,
    val url: String,
    val kind: RuleKind,
    val entries: Int,
    val updatedAtMs: Long,
    val error: String?,
    val name: String = "",
    val title: String = url,
)

data class RoutingUiState(
    val loaded: Boolean = false,
    val directRules: Int = 0,
    val blockRules: Int = 0,
    val proxyRules: Int = 0,
    val dpiRules: Int = 0,
    val subscriptions: List<SubscriptionUi> = emptyList(),
    val directResolver: String = "",
    val mtu: Int = AppSettings.DEFAULT_MTU,
    val busy: Boolean = false,
    val message: String? = null,
)

enum class RoutingInputError { Resolver, Mtu, Link, Name }

@HiltViewModel
class RoutingViewModel @Inject constructor(
    private val settings: SettingsRepository,
    private val rules: RulesRepository,
    private val subscriptions: SubscriptionRepository,
) : ViewModel() {
    private val loaded = MutableStateFlow(false)
    private val busy = MutableStateFlow(false)
    private val message = MutableStateFlow<String?>(null)

    init {
        viewModelScope.launch {
            settings.awaitReady()
            loaded.value = true
        }
        viewModelScope.launch { runCatching { subscriptions.refreshStale() } }
    }

    val uiState: StateFlow<RoutingUiState> = combine(settings.settings, loaded, busy, message) { s, ready, working, notice ->
        RoutingUiState(
            loaded = ready,
            directRules = countOf(s.directRules, true),
            blockRules = countOf(s.blockRules, false),
            proxyRules = countOf(s.proxyRules, true),
            dpiRules = countOf(s.dpiRules, true),
            subscriptions = s.subscriptions.map {
                SubscriptionUi(it.id, it.url, it.kind, it.entries, it.updatedAtMs, it.error, it.name, it.title)
            },
            directResolver = s.directResolver,
            mtu = s.mtu,
            busy = working,
            message = notice,
        )
    }.flowOn(Dispatchers.Default).stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), RoutingUiState())

    private fun countOf(text: String, allowIp: Boolean) =
        if (text.isBlank()) 0 else rules.validate(text, allowIp).getOrDefault(RuleCounts()).total

    fun setResolver(value: String): RoutingInputError? {
        if (rules.validateResolver(value).isFailure) return RoutingInputError.Resolver
        viewModelScope.launch { settings.update { it.copy(directResolver = value.trim()) } }
        return null
    }

    fun setMtu(value: String): RoutingInputError? {
        val mtu = value.trim().toIntOrNull()
        if (mtu == null || mtu !in AppSettings.MIN_MTU..AppSettings.MAX_MTU) return RoutingInputError.Mtu
        viewModelScope.launch { settings.update { it.copy(mtu = mtu) } }
        return null
    }

    fun addSubscription(url: String, kind: RuleKind, name: String = ""): RoutingInputError? {
        val link = url.trim()
        if (link.length !in 1..2048 || !(link.startsWith("https://") || link.startsWith("http://"))) {
            return RoutingInputError.Link
        }
        if (!validName(name)) return RoutingInputError.Name
        work {
            subscriptions.add(link, kind, name.trim()).onFailure { message.value = it.message ?: "Cannot add the subscription" }
        }
        return null
    }

    fun editSubscription(id: String, name: String, kind: RuleKind): RoutingInputError? {
        if (!validName(name)) return RoutingInputError.Name
        work {
            subscriptions.edit(id, name.trim(), kind)
                .onSuccess { saved -> saved.error?.let { message.value = it } }
                .onFailure { message.value = it.message ?: "Cannot save the subscription" }
        }
        return null
    }

    private fun validName(name: String) = name.trim().let { it.length <= RuleSubscription.MAX_NAME && it.none(Char::isISOControl) }

    fun refreshSubscription(id: String) = work {
        subscriptions.refresh(id).onFailure { message.value = it.message ?: "Cannot update the list" }
    }

    fun refreshAll() = work {
        subscriptions.refreshAll().onFailure { message.value = it.message ?: "Cannot update the lists" }
    }

    fun removeSubscription(id: String) = work { subscriptions.remove(id) }

    fun consumeMessage() {
        message.value = null
    }

    private fun work(block: suspend () -> Unit) {
        viewModelScope.launch {
            if (busy.value) return@launch
            busy.value = true
            try {
                block()
            } finally {
                busy.value = false
            }
        }
    }
}

data class RuleEditorUiState(
    val loaded: Boolean = false,
    val text: String = "",
    val counts: RuleCounts = RuleCounts(),
    val dirty: Boolean = false,
    val saving: Boolean = false,
    val error: String? = null,
    val imported: Boolean = false,
)

@HiltViewModel(assistedFactory = RuleEditorViewModel.Factory::class)
class RuleEditorViewModel @AssistedInject constructor(
    @Assisted private val kind: RuleKind,
    private val settings: SettingsRepository,
    private val rules: RulesRepository,
) : ViewModel() {
    @AssistedFactory
    interface Factory {
        fun create(kind: RuleKind): RuleEditorViewModel
    }

    private val mutableState = MutableStateFlow(RuleEditorUiState())
    val uiState: StateFlow<RuleEditorUiState> = mutableState
    private val closed = Channel<Unit>(Channel.BUFFERED)
    val closeEvents = closed.receiveAsFlow()

    private val allowIp get() = kind != RuleKind.Block

    private fun saved(settings: AppSettings): String = when (kind) {
        RuleKind.Direct -> settings.directRules
        RuleKind.Block -> settings.blockRules
        RuleKind.Proxy -> settings.proxyRules
        RuleKind.Dpi -> settings.dpiRules
    }

    private fun store(settings: AppSettings, text: String): AppSettings = when (kind) {
        RuleKind.Direct -> settings.copy(directRules = text)
        RuleKind.Block -> settings.copy(blockRules = text)
        RuleKind.Proxy -> settings.copy(proxyRules = text)
        RuleKind.Dpi -> settings.copy(dpiRules = text)
    }

    init {
        viewModelScope.launch {
            settings.awaitReady()
            val text = saved(settings.settings.value)
            val counts = withContext(Dispatchers.Default) { rules.validate(text, allowIp).getOrDefault(RuleCounts()) }
            mutableState.update { it.copy(loaded = true, text = text, counts = counts) }
        }
    }

    fun onText(value: String) {
        mutableState.update { it.copy(text = value, dirty = true, error = null, imported = false) }
    }

    fun consumeImported() = mutableState.update { it.copy(imported = false) }

    fun save() {
        val state = mutableState.value
        if (state.saving) return
        mutableState.update { it.copy(saving = true) }
        viewModelScope.launch {
            val checked = withContext(Dispatchers.Default) { rules.validate(state.text, allowIp) }
            checked.onSuccess { counts ->
                settings.update { store(it, state.text) }
                mutableState.update { it.copy(saving = false, counts = counts, dirty = false) }
                closed.send(Unit)
            }.onFailure { error ->
                mutableState.update { it.copy(saving = false, error = error.message.orEmpty()) }
            }
        }
    }

    fun import(uri: String) {
        viewModelScope.launch {
            val text = rules.read(uri).getOrElse { error ->
                mutableState.update { it.copy(error = error.message.orEmpty()) }
                return@launch
            }
            val existing = mutableState.value.text
            val joined = if (existing.isBlank()) text else existing.trimEnd() + "\n" + text
            val checked = withContext(Dispatchers.Default) { rules.validate(joined, allowIp) }
            checked.onSuccess { counts ->
                mutableState.update { it.copy(text = joined, counts = counts, dirty = true, error = null, imported = true) }
            }.onFailure { error ->
                mutableState.update { it.copy(error = error.message.orEmpty()) }
            }
        }
    }
}
