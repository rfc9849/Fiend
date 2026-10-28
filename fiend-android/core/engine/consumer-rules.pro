-keep class com.sun.jna.** { *; }
-dontwarn java.awt.Component
-dontwarn java.awt.GraphicsEnvironment
-dontwarn java.awt.HeadlessException
-dontwarn java.awt.Window
-keep class dev.fiend.core.engine.bindings.** { *; }
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}
