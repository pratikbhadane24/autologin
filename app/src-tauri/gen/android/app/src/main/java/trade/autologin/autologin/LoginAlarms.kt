package trade.autologin.autologin

import android.Manifest
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat

/**
 * Daily login alarms on phones. Rust computes the upcoming run times
 * (scheduler.rs) and hands them over; this keeps them across reboots,
 * arms the earliest, and at fire time either posts a "tap to log in"
 * notification or, in automatic mode, also opens AutoLogin straight away.
 */
object LoginAlarms {
  const val EXTRA_RUN = "trade.autologin.autologin.RUN_SCHEDULED"
  private const val PREFS = "login_alarms"
  private const val KEY_TIMES = "times"
  private const val KEY_AUTOMATIC = "automatic"
  private const val KEY_PENDING = "pending_run"
  private const val KEY_FIRED_AT = "fired_at"
  /** A tap on an old notification (or a forged intent) can't start a run. */
  private const val RUN_REQUEST_WINDOW_MS = 2 * 60 * 60 * 1000L
  private const val CHANNEL = "scheduled_login"
  private const val NOTIFICATION_ID = 845
  /** Without exact-alarm access, fire within this window instead. */
  private const val INEXACT_WINDOW_MS = 5 * 60 * 1000L

  fun save(context: Context, times: List<Long>, automatic: Boolean) {
    val prefs = prefs(context)
    val joined = times.sorted().joinToString(",")
    // Rust re-sends the schedule often; only re-arm when it changed.
    if (prefs.getString(KEY_TIMES, null) == joined && prefs.getBoolean(KEY_AUTOMATIC, false) == automatic) return
    prefs.edit().putString(KEY_TIMES, joined).putBoolean(KEY_AUTOMATIC, automatic).apply()
    arm(context)
  }

  /** Arms the earliest future time, or cancels when there is none. */
  fun arm(context: Context) {
    val now = System.currentTimeMillis()
    val next = times(context).firstOrNull { it > now }
    val alarms = context.getSystemService(AlarmManager::class.java)
    val fire = firePendingIntent(context)
    alarms.cancel(fire)
    if (next == null) return
    try {
      if (canScheduleExact(context)) {
        alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, fire)
        return
      }
    } catch (_: SecurityException) {
      // Exact-alarm access was revoked after the check: fall through.
    }
    alarms.setWindow(AlarmManager.RTC_WAKEUP, next, INEXACT_WINDOW_MS, fire)
  }

  fun onAlarm(context: Context) {
    prefs(context).edit().putLong(KEY_FIRED_AT, System.currentTimeMillis()).apply()
    arm(context)
    val automatic = prefs(context).getBoolean(KEY_AUTOMATIC, false)
    notifyRunDue(context, automatic)
    // Opening an activity from the background is only allowed with
    // "Display over other apps"; otherwise the notification is the way in.
    if (automatic && Settings.canDrawOverlays(context)) {
      context.startActivity(runIntent(context).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
  }

  /**
   * Called when AutoLogin is opened by the alarm or its notification. The
   * activity is exported, so only an alarm that really fired recently, and
   * hasn't been used yet, turns into a run.
   */
  @Synchronized
  fun requestRun(context: Context) {
    val prefs = prefs(context)
    val firedAt = prefs.getLong(KEY_FIRED_AT, 0)
    if (System.currentTimeMillis() - firedAt !in 0..RUN_REQUEST_WINDOW_MS) return
    prefs.edit().putBoolean(KEY_PENDING, true).putLong(KEY_FIRED_AT, 0).commit()
  }

  /** True once per alarm: the app should start the scheduled run now. */
  @Synchronized
  fun takePendingRun(context: Context): Boolean {
    val prefs = prefs(context)
    if (!prefs.getBoolean(KEY_PENDING, false)) return false
    prefs.edit().putBoolean(KEY_PENDING, false).commit()
    NotificationManagerCompat.from(context).cancel(NOTIFICATION_ID)
    return true
  }

  fun canScheduleExact(context: Context): Boolean =
    Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
      context.getSystemService(AlarmManager::class.java).canScheduleExactAlarms()

  fun canNotify(context: Context): Boolean =
    Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
      ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
      PackageManager.PERMISSION_GRANTED

  private fun notifyRunDue(context: Context, automatic: Boolean) {
    if (!canNotify(context)) return
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
      NotificationChannel(CHANNEL, "Daily login", NotificationManager.IMPORTANCE_HIGH).apply {
        description = "Reminds you to log in to your broker accounts at the time you chose"
      }
    )
    val open = PendingIntent.getActivity(
      context, 0, runIntent(context), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val (title, text) = if (automatic) {
      "Logging in to your broker accounts" to "If AutoLogin didn't open, tap here to start."
    } else {
      "Time to log in" to "Tap to log in to your broker accounts."
    }
    val notification = NotificationCompat.Builder(context, CHANNEL)
      .setSmallIcon(R.drawable.ic_stat_autologin)
      .setContentTitle(title)
      .setContentText(text)
      .setContentIntent(open)
      .setAutoCancel(true)
      .setCategory(NotificationCompat.CATEGORY_REMINDER)
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .build()
    try {
      NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, notification)
    } catch (_: SecurityException) {
      // Permission revoked between the check and the post: nothing to show.
    }
  }

  private fun runIntent(context: Context): Intent =
    Intent(context, MainActivity::class.java)
      .putExtra(EXTRA_RUN, true)
      .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)

  private fun firePendingIntent(context: Context): PendingIntent =
    PendingIntent.getBroadcast(
      context, 0, Intent(context, AlarmReceiver::class.java),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )

  private fun times(context: Context): List<Long> =
    prefs(context).getString(KEY_TIMES, "").orEmpty().split(",").mapNotNull { it.toLongOrNull() }

  private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

class AlarmReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) = LoginAlarms.onAlarm(context)
}

/** Alarms don't survive a reboot, an app update or a clock change. */
class RearmReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) = LoginAlarms.arm(context)
}
