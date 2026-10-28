package dev.fiend.feature.settings

import dev.fiend.core.data.RulesRepository
import dev.fiend.core.data.SettingsRepository
import dev.fiend.core.model.AppSettings
import dev.fiend.core.model.RuleCounts
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
        override fun validateResolver(value: String) =
            if (value.isBlank() || value.trim().first().isDigit()) Result.success(Unit) else Result.failure(IllegalArgumentException("resolver"))
        override suspend fun read(uri: String) = imported
    }

    private suspend fun routing(model: RoutingViewModel, predicate: (RoutingUiState) -> Boolean = { it.loaded }) =
        withTimeout(5_000) { model.uiState.first(predicate) }

    private suspend fun editor(model: RuleEditorViewModel) = withTimeout(5_000) { model.uiState.first { it.loaded } }

    @Test
    fun routingScreenShowsSavedCountsAndValues() = runBlocking {
        val settings = FakeSettings(AppSettings(directRules = "a.example\nip:1.0.0.0/8", blockRules = "ads.example", mtu = 1400, directResolver = "9.9.9.9", directMode = true))
        val state = routing(RoutingViewModel(settings, FakeRules())) { it.loaded && it.directRules == 2 }
        assertEquals(2, state.directRules)
        assertEquals(1, state.blockRules)
        assertEquals(1400, state.mtu)
        assertEquals("9.9.9.9", state.directResolver)
        assertTrue(state.directMode)
    }

    @Test
    fun resolverAndMtuAreValidatedBeforeTheyAreSaved() = runBlocking {
        val settings = FakeSettings()
        val model = RoutingViewModel(settings, FakeRules())
        routing(model)
        assertEquals(RoutingInputError.Resolver, model.setResolver("dns.example"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("1279"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("9001"))
        assertEquals(RoutingInputError.Mtu, model.setMtu("abc"))
        assertEquals("", settings.settings.value.directResolver)
        assertEquals(AppSettings.DEFAULT_MTU, settings.settings.value.mtu)
        assertNull(model.setResolver(" 9.9.9.9 "))
        assertNull(model.setMtu("1280"))
        model.setDirectMode(true)
        assertEquals("9.9.9.9", settings.settings.value.directResolver)
        assertEquals(1280, settings.settings.value.mtu)
        assertTrue(settings.settings.value.directMode)
    }

    @Test
    fun editorSavesValidListsAndKeepsBadOnes() = runBlocking {
        val settings = FakeSettings(AppSettings(directRules = "a.example"))
        val model = RuleEditorViewModel(false, settings, FakeRules())
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
        val model = RuleEditorViewModel(true, settings, FakeRules())
        editor(model)
        model.onText("ip:1.0.0.0/8")
        model.save()
        withTimeout(5_000) { model.uiState.first { it.error != null } }
        assertEquals("", settings.settings.value.blockRules)
    }

    @Test
    fun importAppendsValidatedTextAndReportsFailures() = runBlocking {
        val rules = FakeRules(Result.success("b.example\nc.example"))
        val model = RuleEditorViewModel(false, FakeSettings(AppSettings(directRules = "a.example")), rules)
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
