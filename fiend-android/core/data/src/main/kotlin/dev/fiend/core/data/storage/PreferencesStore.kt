package dev.fiend.core.data.storage

import android.content.Context
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStoreFile
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.data.ApplicationScope
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers

@Singleton
internal class PreferencesStore @Inject constructor(@ApplicationContext context: Context, @ApplicationScope scope: CoroutineScope) {
    val data = PreferenceDataStoreFactory.create(scope = CoroutineScope(scope.coroutineContext + Dispatchers.IO)) { context.preferencesDataStoreFile("fiend") }
    companion object { val ACTIVE_PROFILE = stringPreferencesKey("active_profile_id") }
}
