package dev.fiend.feature.settings

import dev.fiend.core.data.RulesRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.data.SubscriptionRepository
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.RuleCounts
import dev.fiend.core.model.RuleFileSpec
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class RoutingViewModelTest {
    @Before fun prepare() { Dispatchers.setMain(UnconfinedTestDispatcher()) }
    @After fun finish() { Dispatchers.resetMain() }

    private class FakeSettings(initial: AppSettings = AppSettings()) : SettingsRepository {
        override val settings = MutableStateFlow(initial)
        override suspend fun update(transform: (AppSettings) -> AppSettings) { settings.value = transform(settings.value) }
    }

    private class FakeRules(var imported: Result<String> = Result.success("")) : RulesRepository {
        override fun validate(text: String, allowIp: Boolean): Result<RuleCounts> {
            val lines = text.lines().map { it.trim() }.filter { it.isNotEmpty() }
            if (lines.any { it.startsWith("bad") }) return Result.failure(IllegalArgumentException("line 1: entry type is not known"))
            if (!allowIp && lines.any { it.startsWith("ip:") }) return Result.failure(IllegalArgumentException("ip entries are not allowed"))
            return Result.success(RuleCounts(domains = lines.count { !it.startsWith("ip:") }, cidrs = lines.count { it.startsWith("ip:") }))
        }
        override fun inspectFile(path: String, kind: RuleKind): Result<RuleCounts> =
            if (path.startsWith("bad")) Result.failure(IllegalArgumentException("Cannot read the list")) else Result.success(RuleCounts(domains = 3))
        override fun validateResolver(value: String) =
            if (value.isBlank() || value.trim().first().isDigit()) Result.success(Unit) else Result.failure(IllegalArgumentException("resolver"))
        override suspend fun read(uri: String) = imported
    }

    private class FakeSubscriptions(private val settings: SettingsRepository? = null) : SubscriptionRepository {
        var stale = 0
        val added = mutableListOf<RuleSubscription>()
        var failWith: Throwable? = null
        override suspend fun refreshStale() { stale += 1 }
        override suspend fun refresh(id: String): Result<Unit> = failWith?.let { Result.failure(it) } ?: Result.success(Unit)
        override suspend fun refreshAll(): Result<Unit> = failWith?.let { Result.failure(it) } ?: Result.success(Unit)
        override suspend fun add(url: String, kind: RuleKind, name: String): Result<RuleSubscription> {
            val subscription = RuleSubscription("id-${added.size}", url, kind, updatedAtMs = 1, entries = 3, name = name)
            added += subscription
            settings?.update { it.copy(subscriptions = it.subscriptions + subscription) }
            return Result.success(subscription)
        }
        override suspend fun edit(id: String, name: String, kind: RuleKind): Result<RuleSubscription> {
            val index = added.indexOfFirst { it.id == id }
            if (index < 0) return Result.failure(IllegalArgumentException("The subscription is not available"))
            val changed = added[index].copy(name = name, kind = kind)
            added[index] = changed
            settings?.update { current -> current.copy(subscriptions = current.subscriptions.map { if (it.id == id) changed else it }) }
            return Result.success(changed)
        }
        override suspend fun remove(id: String) { added.removeAll { it.id == id } }
        override suspend fun ruleFiles(): List<RuleFileSpec> = emptyList()
    }

    private suspend fun routing(model: RoutingViewModel, predicate: (RoutingUiState) -> Boolean = { it.loaded }) =
        withTimeout(5_000) { model.uiState.first(predicate) }

    private suspend fun editor(model: RuleEditorViewModel) = withTimeout(5_000) { model.uiState.first { it.loaded } }

    @Test
    fun routingScreenShowsSavedCountsAndValues() = runBlocking {
        val settings = FakeSettings(AppSettings(directRules = "a.example\nip:1.0.0.0/8", blockRules = "ads.example", mtu = 1400, directResolver = "9.9.9.9"))
        val state = routing(RoutingViewModel(settings, FakeRules(), FakeSubscriptions())) { it.loaded && it.directRules == 2 }
        assertEquals(2, state.directRules)
        assertEquals(1, state.blockRules)
        assertEquals(1400, state.mtu)
        assertEquals("9.9.9.9", state.directResolver)
    }

    @Test
    fun resolverAndMtuAreValidatedBeforeTheyAreSaved() = runBlocking {
        val settings = FakeSettings()
        val model = RoutingViewModel(settings, FakeRules(), FakeSubscriptions())
        routing(model)
        assertEquals(RoutingInputError.Resolver, model.setResolver("dns.example"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("1279"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("9001"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("abc"))
        assertEquals("", settings.settings.value.directResolver)
        assertEquals(AppSettings.DEFAULT_MTU, settings.settings.value.mtu)
        assertNull(model.setResolver(" 9.9.9.9 "))
        assertNull(model.setMtu("1280"))
        assertEquals("9.9.9.9", settings.settings.value.directResolver)
        assertEquals(1280, settings.settings.value.mtu)
    }

    @Test
    fun editorSavesValidListsAndKeepsBadOnes() = runBlocking {
        val settings = FakeSettings(AppSettings(directRules = "a.example"))
        val model = RuleEditorViewModel(RuleKind.Direct, settings, FakeRules())
        assertEquals("a.example", editor(model).text)
        model.onText("bad:x")
        assertTrue(model.uiState.value.dirty)
        model.save()
        val failed = withTimeout(5_000) { model.uiState.first { it.error != null } }
        assertEquals("line 1: entry type is not known", failed.error)
        assertEquals("a.example", settings.settings.value.directRules)
        model.onText("b.example\nip:1.0.0.0/8")
        assertNull(model.uiState.value.error)
        model.save()
        withTimeout(5_000) { model.uiState.first { !it.dirty } }
        assertEquals("b.example\nip:1.0.0.0/8", settings.settings.value.directRules)
        assertEquals(RuleCounts(domains = 1, cidrs = 1), model.uiState.value.counts)
    }

    @Test
    fun blockEditorRejectsAddressRules() = runBlocking {
        val settings = FakeSettings()
        val model = RuleEditorViewModel(RuleKind.Block, settings, FakeRules())
        editor(model)
        model.onText("ip:1.0.0.0/8")
        model.save()
        withTimeout(5_000) { model.uiState.first { it.error != null } }
        assertEquals("", settings.settings.value.blockRules)
    }

    @Test
    fun subscriptionsRefreshOnStartAndRejectBadLinks() = runBlocking {
        val settings = FakeSettings()
        val subscriptions = FakeSubscriptions(settings)
        val model = RoutingViewModel(settings, FakeRules(), subscriptions)
        routing(model)
        assertEquals(1, subscriptions.stale)
        assertEquals(RoutingInputError.Link, model.addSubscription("", RuleKind.Direct))
        assertEquals(RoutingInputError.Link, model.addSubscription("ftp://example.com/list.srs", RuleKind.Proxy))
        assertTrue(subscriptions.added.isEmpty())
        assertNull(model.addSubscription(" https://example.com/list.srs ", RuleKind.Proxy))
        withTimeout(5_000) { model.uiState.first { it.subscriptions.isNotEmpty() } }
        assertEquals("https://example.com/list.srs", subscriptions.added.single().url)
        assertEquals(RuleKind.Proxy, subscriptions.added.single().kind)
        assertEquals(3, model.uiState.value.subscriptions.single().entries)
        assertEquals("list", model.uiState.value.subscriptions.single().title)
        withTimeout(5_000) { model.uiState.first { !it.busy } }
        assertEquals(RoutingInputError.Name, model.editSubscription("id-0", "x".repeat(65), RuleKind.Dpi))
        assertNull(model.editSubscription("id-0", " Blocked ", RuleKind.Dpi))
        val edited = withTimeout(5_000) { model.uiState.first { it.subscriptions.single().kind == RuleKind.Dpi } }
        assertEquals("Blocked", edited.subscriptions.single().title)
        withTimeout(5_000) { model.uiState.first { !it.busy } }
        subscriptions.failWith = IllegalArgumentException("HTTP 404")
        model.refreshSubscription("id-0")
        val notice = withTimeout(5_000) { model.uiState.first { it.message != null } }
        assertEquals("HTTP 404", notice.message)
        model.consumeMessage()
        withTimeout(5_000) { model.uiState.first { it.message == null && !it.busy } }
        Unit
    }

    @Test
    fun importAppendsValidatedTextAndReportsFailures() = runBlocking {
        val rules = FakeRules(Result.success("b.example\nc.example"))
        val model = RuleEditorViewModel(RuleKind.Direct, FakeSettings(AppSettings(directRules = "a.example")), rules)
        editor(model)
        model.import("content://list")
        val state = withTimeout(5_000) { model.uiState.first { it.imported } }
        assertEquals("a.example\nb.example\nc.example", state.text)
        assertEquals(3, state.counts.total)
        assertTrue(state.dirty)
        model.consumeImported()
        assertFalse(model.uiState.value.imported)
        rules.imported = Result.failure(IllegalArgumentException("The file is larger than 1 MiB"))
        model.import("content://large")
        assertEquals("The file is larger than 1 MiB", model.uiState.value.error)
        assertEquals("a.example\nb.example\nc.example", model.uiState.value.text)
    }
}
