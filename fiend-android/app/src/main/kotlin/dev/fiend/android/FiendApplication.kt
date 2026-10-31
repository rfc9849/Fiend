package dev.fiend.android

import android.app.Application
import dagger.hilt.android.HiltAndroidApp
import javax.inject.Inject

@HiltAndroidApp
class FiendApplication : Application() {
    @Inject lateinit var tileStateUpdater: TileStateUpdater
}
