package dev.fiend.feature.home

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import dev.fiend.core.designsystem.theme.FiendTheme
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats

private val previewProfiles = listOf(Profile("preview", "Preview server", "192.0.2.1", 443, "42".repeat(32), "preview"))
private val previewTemplate = FingerprintTemplate("preview", "Captured template", "TLS")

@Preview(showBackground = true)
@Composable
private fun HomeConnectedPreview() {
    FiendTheme {
        HomeScreen(HomeUiState(TunnelState.Connected("preview", System.currentTimeMillis() - 754_000),
            TunnelStats(12_400_000, 1_800_000, 224_000_000, 18_000_000, 38, 21), previewProfiles.first(), previewTemplate, previewProfiles), {}, {}, {}, {})
    }
}

@Preview(showBackground = true)
@Composable
private fun HomeIdlePreview() {
    FiendTheme {
        HomeScreen(HomeUiState(activeProfile = previewProfiles.first(), activeTemplate = previewTemplate, profiles = previewProfiles), {}, {}, {}, {})
    }
}
