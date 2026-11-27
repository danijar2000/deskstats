package app.deskstats

import android.app.Activity
import android.app.AlertDialog
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.Settings as SystemSettings
import android.widget.Toast
import org.json.JSONObject
import java.io.File
import java.io.FileNotFoundException
import java.net.HttpURLConnection
import java.net.URL

/**
 * Updates from GitHub Releases: the app is in no store, so it asks GitHub for the latest release itself, and when
 * that is newer, offers to download its APK and open the system installer with it.
 */
object Updater {
    private const val LATEST = "https://api.github.com/repos/danijar2000/deskstats/releases/latest"
    private const val PREFS = "update"
    private const val KEY_CHECKED = "checked"
    private const val KEY_SKIPPED = "skipped"
    /** Asked at most this often on its own; a tap on the version in the settings asks at once. */
    private const val EVERY_MS = 6 * 60 * 60 * 1000L
    private const val FILE = "update.apk"

    class Release(val version: String, val url: String, val size: Long)

    private val main = Handler(Looper.getMainLooper())
    @Volatile private var busy = false

    fun installed(context: Context): String = SettingsPage.appVersion(context)

    /** Segment by segment, so that 0.10.0 is newer than 0.9.3. Missing segments count as 0. */
    fun newer(remote: String, local: String): Boolean {
        val r = remote.removePrefix("v").split(".").map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
        val l = local.removePrefix("v").split(".").map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
        for (i in 0 until maxOf(r.size, l.size)) {
            val a = r.getOrElse(i) { 0 }
            val b = l.getOrElse(i) { 0 }
            if (a != b) return a > b
        }
        return false
    }

    private fun get(url: String, timeout: Int): HttpURLConnection {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = timeout
        c.readTimeout = timeout
        c.instanceFollowRedirects = true
        c.setRequestProperty("Accept", "application/vnd.github+json")
        c.setRequestProperty("User-Agent", "Deskstats-UpdateCheck")
        return c
    }

    /** The latest release, when it is newer than this app; null when it is not. Throws when GitHub cannot be asked. */
    private fun latest(context: Context): Release? {
        val c = get(LATEST, 10_000)
        try {
            // 404: no release yet, so nothing to update to.
            if (c.responseCode == 404) return null
            if (c.responseCode != 200) throw IllegalStateException("GitHub: HTTP ${c.responseCode}")
            val json = JSONObject(c.inputStream.bufferedReader().use { it.readText() })
            val version = json.optString("tag_name").removePrefix("v")
            if (version.isEmpty() || !newer(version, installed(context))) return null
            val assets = json.optJSONArray("assets") ?: return null
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                if (a.optString("name") == "deskstats.apk") return Release(version, a.optString("browser_download_url"), a.optLong("size"))
            }
            return null
        } finally {
            c.disconnect()
        }
    }

    /**
     * Asks GitHub in the background and offers the update in a dialog. On its own ([manual] false) it asks at most
     * every few hours and keeps quiet about a version the user put off; asked from the settings it always answers.
     */
    fun check(activity: Activity, ui: Context, manual: Boolean) {
        val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val now = System.currentTimeMillis()
        if (busy || (!manual && now - prefs.getLong(KEY_CHECKED, 0L) in 0 until EVERY_MS)) return
        busy = true
        Thread({
            var problem: String? = null
            val release = try {
                latest(activity)
            } catch (e: Exception) {
                problem = e.message ?: e.javaClass.simpleName
                null
            }
            prefs.edit().putLong(KEY_CHECKED, now).apply()
            main.post {
                busy = false
                if (gone(activity)) return@post
                when {
                    release != null && (manual || prefs.getString(KEY_SKIPPED, "") != release.version) -> offer(activity, ui, release)
                    // Asked by hand: say what happened, and never call a version the latest without having asked.
                    manual && problem != null -> Toast.makeText(activity, ui.getString(R.string.update_failed, problem), Toast.LENGTH_LONG).show()
                    manual -> Toast.makeText(activity, ui.getString(R.string.update_latest, installed(activity)), Toast.LENGTH_SHORT).show()
                }
            }
        }, "update-check").start()
    }

    private fun offer(activity: Activity, ui: Context, release: Release) {
        val megabytes = String.format(Ui.locale, "%.1f", release.size / 1_048_576.0)
        AlertDialog.Builder(activity)
            .setTitle(ui.getString(R.string.update_title, release.version))
            .setMessage(ui.getString(R.string.update_message, installed(activity), megabytes))
            .setPositiveButton(ui.getString(R.string.update_now)) { _, _ -> start(activity, ui, release) }
            .setNegativeButton(ui.getString(R.string.update_later)) { _, _ ->
                activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY_SKIPPED, release.version).apply()
            }
            .show()
    }

    private fun start(activity: Activity, ui: Context, release: Release) {
        // Android 8 and later want the user to let this app install apps; then the update is offered again.
        if (Build.VERSION.SDK_INT >= 26 && !activity.packageManager.canRequestPackageInstalls()) {
            Toast.makeText(activity, ui.getString(R.string.update_allow), Toast.LENGTH_LONG).show()
            activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putLong(KEY_CHECKED, 0L).remove(KEY_SKIPPED).apply()
            try {
                activity.startActivity(Intent(SystemSettings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}")))
            } catch (e: Exception) {
            }
            return
        }
        // The installer reads the file itself: below Android 7 it gets a file:// address, which works only for a file
        // on the shared storage, not in the app's private folder.
        val dir = activity.getExternalFilesDir(null) ?: if (Build.VERSION.SDK_INT >= 24) activity.filesDir else null
        if (dir == null) {
            Toast.makeText(activity, ui.getString(R.string.update_failed, ui.getString(R.string.update_no_storage)), Toast.LENGTH_LONG).show()
            return
        }
        val progress = AlertDialog.Builder(activity)
            .setTitle(ui.getString(R.string.update_title, release.version))
            .setMessage(ui.getString(R.string.update_downloading, 0))
            .setCancelable(false)
            .show()
        val file = File(dir, FILE)
        Thread({
            val error = try {
                download(release.url, file) { percent -> main.post { if (!gone(activity)) progress.setMessage(ui.getString(R.string.update_downloading, percent)) } }
                null
            } catch (e: Exception) {
                e.message ?: e.javaClass.simpleName
            }
            main.post {
                // The activity may have closed while the file came: its dialog and its window are gone then.
                if (gone(activity)) return@post
                try {
                    if (progress.isShowing) progress.dismiss()
                } catch (e: IllegalArgumentException) {
                }
                if (error != null) {
                    Toast.makeText(activity, ui.getString(R.string.update_failed, error), Toast.LENGTH_LONG).show()
                } else {
                    install(activity, ui, file)
                }
            }
        }, "update-download").start()
    }

    private fun download(url: String, file: File, progress: (Int) -> Unit) {
        val c = get(url, 30_000)
        try {
            if (c.responseCode != 200) throw IllegalStateException("HTTP ${c.responseCode}")
            val total = c.contentLength.toLong()
            var done = 0L
            var shown = -1
            file.delete()
            c.inputStream.use { input ->
                file.outputStream().use { output ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        output.write(buf, 0, n)
                        done += n
                        val percent = if (total > 0) (done * 100 / total).toInt() else 0
                        if (percent != shown) {
                            shown = percent
                            progress(percent)
                        }
                    }
                }
            }
            if (total > 0 && done != total) throw IllegalStateException("$done / $total")
        } finally {
            c.disconnect()
        }
    }

    private fun gone(activity: Activity) = activity.isFinishing || (Build.VERSION.SDK_INT >= 17 && activity.isDestroyed)

    private fun install(activity: Activity, ui: Context, file: File) {
        // Android 7 and later do not let a file:// address leave the app: the installer gets a content:// one from
        // ApkProvider, with the right to read it.
        val uri = if (Build.VERSION.SDK_INT >= 24) Uri.parse("content://${ApkProvider.authority(activity)}/$FILE") else Uri.fromFile(file)
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try {
            activity.startActivity(intent)
        } catch (e: Exception) {
            Toast.makeText(activity, ui.getString(R.string.update_failed, e.message ?: ""), Toast.LENGTH_LONG).show()
        }
    }
}

/** Hands the downloaded update to the system installer, and nothing else. */
class ApkProvider : ContentProvider() {
    override fun onCreate() = true

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val context = context ?: throw FileNotFoundException()
        if (uri.lastPathSegment != "update.apk" || mode != "r") throw FileNotFoundException()
        val file = File(context.getExternalFilesDir(null) ?: context.filesDir, "update.apk")
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri) = "application/vnd.android.package-archive"
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, order: String?): Cursor? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = 0

    companion object {
        fun authority(context: Context) = "${context.packageName}.apk"
    }
}
