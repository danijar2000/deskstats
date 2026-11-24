package app.deskstats

import android.content.Context
import android.content.ContextWrapper
import android.content.SharedPreferences
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.content.res.Resources
import android.graphics.Paint
import android.os.Build
import java.util.Locale

class Palette(
    val background: Int,
    val card: Int,
    val raised: Int,
    val border: Int,
    val grid: Int,
    val text: Int,
    val soft: Int,
    val muted: Int,
    val dim: Int,
    val chevron: Int,
    val pressed: Int,
    val pressedFill: Int,
    val accent: Int,
    val selection: Int,
    val good: Int,
    val bad: Int,
    val warn: Int,
    val metrics: IntArray,
    val up: Int,
    val clock: Int,
    val seconds: Int,
    val scrollMark: Int,
    val switchOff: Int,
    val knobOn: Int,
    val knobOff: Int,
    val chartAlpha: Int,
    val glow: Boolean,
    val sun: Int,
    val moon: Int,
    val cloud: Int,
    val rainCloud: Int,
    val staleCloud: Int,
    val swatch: IntArray,
    val chipOn: Int,
    val chipOnText: Int,
) {
    companion object {
        val DARK = Palette(
            background = 0xFF06070A.toInt(),
            card = 0xFF111318.toInt(),
            raised = 0xFF1A1D24.toInt(),
            border = 0xFF262A32.toInt(),
            grid = 0xFF1C2027.toInt(),
            text = 0xFFFFFFFF.toInt(),
            soft = 0xFFD5D9E0.toInt(),
            muted = 0xFFA0A8B5.toInt(),
            dim = 0xFF7C8494.toInt(),
            chevron = 0xFF596170.toInt(),
            pressed = 0xFF6B7585.toInt(),
            pressedFill = 0x16FFFFFF,
            accent = 0xFF2DD4BF.toInt(),
            selection = 0xFF2DD4BF.toInt(),
            good = 0xFF22C55E.toInt(),
            bad = 0xFFF87171.toInt(),
            warn = 0xFFF59E0B.toInt(),
            metrics = intArrayOf(0xFF2DD4BF.toInt(), 0xFF84CC16.toInt(), 0xFFA78BFA.toInt(), 0xFFF59E0B.toInt()),
            up = 0xFF60A5FA.toInt(),
            clock = 0xFFFFFFFF.toInt(),
            seconds = 0xFFA0A8B5.toInt(),
            scrollMark = 0xFF3B4250.toInt(),
            switchOff = 0xFF262A32.toInt(),
            knobOn = 0xFF06070A.toInt(),
            knobOff = 0xFFA0A8B5.toInt(),
            chartAlpha = 0x40,
            glow = false,
            sun = 0xFFFACC15.toInt(),
            moon = 0xFFE5E7EB.toInt(),
            cloud = 0xFFCBD5E1.toInt(),
            rainCloud = 0xFF94A3B8.toInt(),
            staleCloud = 0xFF5B6472.toInt(),
            swatch = intArrayOf(0xFF111318.toInt(), 0xFF2DD4BF.toInt()),
            chipOn = 0xFF2DD4BF.toInt(),
            chipOnText = 0xFF06070A.toInt(),
        )

        val LIGHT = Palette(
            background = 0xFFEEF1F5.toInt(),
            card = 0xFFFFFFFF.toInt(),
            raised = 0xFFFFFFFF.toInt(),
            border = 0xFFD9DEE6.toInt(),
            grid = 0xFFE6EAF0.toInt(),
            text = 0xFF0F141B.toInt(),
            soft = 0xFF2B3440.toInt(),
            muted = 0xFF5E6978.toInt(),
            dim = 0xFF8C96A4.toInt(),
            chevron = 0xFFA4ADBA.toInt(),
            pressed = 0xFF7D8899.toInt(),
            pressedFill = 0x12000000,
            accent = 0xFF0F8F82.toInt(),
            selection = 0xFF0F9D8F.toInt(),
            good = 0xFF16A34A.toInt(),
            bad = 0xFFDC2626.toInt(),
            warn = 0xFFD97706.toInt(),
            metrics = intArrayOf(0xFF0D9488.toInt(), 0xFF4D9A0F.toInt(), 0xFF7C3AED.toInt(), 0xFFE07B00.toInt()),
            up = 0xFF2563EB.toInt(),
            clock = 0xFF0F141B.toInt(),
            seconds = 0xFF7A8594.toInt(),
            scrollMark = 0xFFC3CAD4.toInt(),
            switchOff = 0xFFD5DBE3.toInt(),
            knobOn = 0xFFFFFFFF.toInt(),
            knobOff = 0xFFFFFFFF.toInt(),
            chartAlpha = 0x38,
            glow = false,
            sun = 0xFFF2A900.toInt(),
            moon = 0xFF8E99A8.toInt(),
            cloud = 0xFFAEB8C6.toInt(),
            rainCloud = 0xFF8794A6.toInt(),
            staleCloud = 0xFFC9D0DA.toInt(),
            swatch = intArrayOf(0xFFF4F6F9.toInt(), 0xFF0D9488.toInt()),
            chipOn = 0xFF0F9D8F.toInt(),
            chipOnText = 0xFFFFFFFF.toInt(),
        )

        val NEON = Palette(
            background = 0xFF000C1D.toInt(),
            card = 0xFF061932.toInt(),
            raised = 0xFF0A2346.toInt(),
            border = 0xFF1F3046.toInt(),
            grid = 0xFF102641.toInt(),
            text = 0xFF00BDC4.toInt(),
            soft = 0xFF3FB6D6.toInt(),
            muted = 0xFF5C7FA3.toInt(),
            dim = 0xFF3D5C7E.toInt(),
            chevron = 0xFF2E5A88.toInt(),
            pressed = 0xFFC31FBC.toInt(),
            pressedFill = 0x2672258E,
            accent = 0xFFED25D5.toInt(),
            selection = 0xFF72258E.toInt(),
            good = 0xFF00BDC4.toInt(),
            bad = 0xFFFD0D23.toInt(),
            warn = 0xFFED25D5.toInt(),
            metrics = intArrayOf(0xFF00BDC4.toInt(), 0xFFED25D5.toInt(), 0xFF498EF2.toInt(), 0xFFC31FBC.toInt()),
            up = 0xFF00BDC4.toInt(),
            clock = 0xFF00BDC4.toInt(),
            seconds = 0xFFC31FBC.toInt(),
            scrollMark = 0xFF044079.toInt(),
            switchOff = 0xFF1F3046.toInt(),
            knobOn = 0xFF000C1D.toInt(),
            knobOff = 0xFF5C7FA3.toInt(),
            chartAlpha = 0x55,
            glow = true,
            sun = 0xFFED25D5.toInt(),
            moon = 0xFF00BDC4.toInt(),
            cloud = 0xFF498EF2.toInt(),
            rainCloud = 0xFF2E5A88.toInt(),
            staleCloud = 0xFF1F3046.toInt(),
            swatch = intArrayOf(0xFF061932.toInt(), 0xFFED25D5.toInt()),
            chipOn = 0xFF72258E.toInt(),
            chipOnText = 0xFFFFE0FA.toInt(),
        )

        fun of(id: String) = when (id) {
            Options.LIGHT -> LIGHT
            Options.NEON -> NEON
            else -> DARK
        }
    }
}

object Ui {
    var p: Palette = Palette.DARK
    var locale: Locale = Locale.ENGLISH
    var language = "en"

    fun glow(paint: Paint, color: Int, radius: Float) {
        if (p.glow && radius > 0f) paint.setShadowLayer(radius, 0f, 0f, (color and 0x00FFFFFF) or 0xA0000000.toInt()) else paint.clearShadowLayer()
    }

    fun noGlow(paint: Paint) {
        if (p.glow) paint.clearShadowLayer()
    }

    fun systemLanguage(): String {
        val conf = Resources.getSystem().configuration
        val locale = if (Build.VERSION.SDK_INT >= 24) conf.locales[0] else @Suppress("DEPRECATION") conf.locale
        val code = locale?.language ?: return "en"
        return if (code in Options.LANGUAGES) code else "en"
    }

    fun localeOf(language: String): Locale = if (language == "zh") Locale.SIMPLIFIED_CHINESE else Locale(language)

    fun localize(base: Context, setting: String): Context {
        language = if (setting.isEmpty()) systemLanguage() else setting
        locale = localeOf(language)
        val conf = Configuration(base.resources.configuration)
        if (Build.VERSION.SDK_INT >= 17) {
            conf.setLocale(locale)
            val res = base.createConfigurationContext(conf).resources
            return object : ContextWrapper(base) {
                override fun getResources(): Resources = res
            }
        }
        @Suppress("DEPRECATION")
        conf.locale = locale
        @Suppress("DEPRECATION")
        base.resources.updateConfiguration(conf, base.resources.displayMetrics)
        return base
    }
}

data class Options(
    val language: String = "",
    val theme: String = DARK,
    val orientation: String = LANDSCAPE,
    val lan: Boolean = false,
    val alerts: Boolean = true,
    val intervalMs: Int = 1000,
    val weatherMin: Int = 60,
) {
    val requestedOrientation: Int
        get() = when (orientation) {
            PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
            AUTO -> ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
            else -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        }

    val query: String
        get() = "/config?interval_ms=$intervalMs&lan=${if (lan) 1 else 0}&alerts=${if (alerts) 1 else 0}&weather_min=$weatherMin"

    fun save(prefs: SharedPreferences) {
        prefs.edit()
            .putString(K_LANGUAGE, language)
            .putString(K_THEME, theme)
            .putString(K_ORIENTATION, orientation)
            .putBoolean(K_LAN, lan)
            .putBoolean(K_ALERTS, alerts)
            .putInt(K_INTERVAL, intervalMs)
            .putInt(K_WEATHER, weatherMin)
            .apply()
    }

    companion object {
        const val DARK = "dark"
        const val LIGHT = "light"
        const val NEON = "neon"
        const val LANDSCAPE = "landscape"
        const val PORTRAIT = "portrait"
        const val AUTO = "auto"
        val LANGUAGES = listOf("en", "ru", "es", "de", "fr", "pt", "zh", "ja")
        val THEMES = listOf(DARK, LIGHT, NEON)
        val ORIENTATIONS = listOf(LANDSCAPE, PORTRAIT, AUTO)
        val INTERVALS = listOf(500, 1000, 2000, 5000)
        val WEATHER = listOf(15, 30, 60, 180)

        private const val K_LANGUAGE = "language"
        private const val K_THEME = "theme"
        private const val K_ORIENTATION = "orientation"
        private const val K_LAN = "lan"
        private const val K_ALERTS = "alerts"
        private const val K_INTERVAL = "interval_ms"
        private const val K_WEATHER = "weather_min"

        fun load(prefs: SharedPreferences): Options {
            val d = Options()
            return Options(
                language = (prefs.getString(K_LANGUAGE, null) ?: "").takeIf { it.isEmpty() || it in LANGUAGES } ?: "",
                theme = prefs.getString(K_THEME, null)?.takeIf { it in THEMES } ?: d.theme,
                orientation = prefs.getString(K_ORIENTATION, null)?.takeIf { it in ORIENTATIONS } ?: d.orientation,
                lan = prefs.getBoolean(K_LAN, d.lan),
                alerts = prefs.getBoolean(K_ALERTS, d.alerts),
                intervalMs = prefs.getInt(K_INTERVAL, d.intervalMs).takeIf { it in INTERVALS } ?: d.intervalMs,
                weatherMin = prefs.getInt(K_WEATHER, d.weatherMin).takeIf { it in WEATHER } ?: d.weatherMin,
            )
        }
    }
}
