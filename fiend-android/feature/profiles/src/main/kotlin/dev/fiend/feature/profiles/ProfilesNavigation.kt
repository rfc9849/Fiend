package dev.fiend.feature.profiles

import androidx.compose.runtime.Composable
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.navigation3.runtime.EntryProviderScope
import androidx.navigation3.runtime.NavKey
import dev.fiend.feature.profiles.detail.ProfileDetailRoute
import dev.fiend.feature.profiles.detail.ProfileDetailViewModel
import dev.fiend.feature.profiles.edit.ProfileEditRoute
import dev.fiend.feature.profiles.edit.ProfileEditViewModel
import dev.fiend.feature.profiles.list.ProfilesRoute
import kotlinx.serialization.Serializable

@Serializable
data object ProfilesKey : NavKey

@Serializable
data class ProfileDetailKey(val profileId: String) : NavKey

@Serializable
data class ProfileEditKey(val profileId: String? = null) : NavKey

fun EntryProviderScope<NavKey>.profilesEntries(
    onOpenProfile: (String) -> Unit,
    onEditProfile: (String?) -> Unit,
    onBack: () -> Unit,
) {
    entry<ProfilesKey> {
        ProfilesRoute(onOpenProfile = onOpenProfile, onAddManually = { onEditProfile(null) })
    }
    entry<ProfileDetailKey> { key ->
        DetailEntry(key, onEdit = { onEditProfile(key.profileId) }, onOpenProfile = onOpenProfile, onBack = onBack)
    }
    entry<ProfileEditKey> { key ->
        EditEntry(key, onBack = onBack)
    }
}

@Composable
private fun DetailEntry(key: ProfileDetailKey, onEdit: () -> Unit, onOpenProfile: (String) -> Unit, onBack: () -> Unit) {
    val viewModel = hiltViewModel<ProfileDetailViewModel, ProfileDetailViewModel.Factory>(
        creationCallback = { it.create(key.profileId) },
    )
    ProfileDetailRoute(viewModel = viewModel, onEdit = onEdit, onOpenProfile = onOpenProfile, onBack = onBack)
}

@Composable
private fun EditEntry(key: ProfileEditKey, onBack: () -> Unit) {
    val viewModel = hiltViewModel<ProfileEditViewModel, ProfileEditViewModel.Factory>(
        creationCallback = { it.create(key.profileId) },
    )
    ProfileEditRoute(viewModel = viewModel, onBack = onBack)
}
