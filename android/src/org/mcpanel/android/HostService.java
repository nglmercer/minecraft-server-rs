package org.mcpanel.android;

import android.app.*;
import android.content.*;
import android.os.*;
import java.lang.Process;
import android.system.Os;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.zip.*;
import org.json.JSONObject;

/** Owns the panel process and keeps network servers alive while the UI is closed. */
public final class HostService extends Service {
    static volatile String url;
    static volatile boolean active;
    static volatile String status = "Starting host…";
    private volatile boolean stopping;
    private volatile Process panel;
    private PowerManager.WakeLock wake;
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private static final String STOP = "org.mcpanel.android.STOP";

    @Override public void onCreate() {
        super.onCreate();
        url = null;
        active = true;
        status = "Starting host…";
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("host", "Minecraft host", NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, HostService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        Notification notification = new Notification.Builder(this, "host")
            .setContentTitle("MCPANEL host running").setContentText("Tap to manage Minecraft servers")
            .setSmallIcon(android.R.drawable.stat_notify_sync).setContentIntent(open).setOngoing(true)
            .addAction(new Notification.Action.Builder(null, "Stop host", stop).build()).build();
        startForeground(1, notification);
        wake = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "mcpanel:host");
        wake.acquire();
        worker.execute(this::runPanel);
    }

    @Override public int onStartCommand(Intent intent, int flags, int id) {
        if (intent != null && STOP.equals(intent.getAction())) {
            stopping = true;
            status = "Stopping servers…";
            Process child = panel;
            if (child != null) child.destroy(); // SIGTERM: guardian saves and stops every world.
        }
        return START_NOT_STICKY;
    }

    private void installRuntime(File runtime) throws Exception {
        String revision;
        try (InputStream input = getAssets().open("runtime.sha256")) {
            revision = new String(input.readAllBytes(), java.nio.charset.StandardCharsets.US_ASCII).trim();
        }
        File stamp = new File(runtime, ".revision");
        if (stamp.isFile() && new String(Files.readAllBytes(stamp.toPath()), java.nio.charset.StandardCharsets.UTF_8).trim().equals(revision)) {
            // Android assigns a new nativeLibraryDir after an APK upgrade.
            // Runtime data may be unchanged, but the executable link must move.
            bindLauncher(runtime);
            return;
        }
        File stage = new File(getFilesDir(), "runtime-staging");
        deleteTree(stage);
        if (!stage.mkdirs()) throw new IOException("Cannot create runtime staging directory");
        try (ZipInputStream zip = new ZipInputStream(getAssets().open("runtime.zip"))) {
            ZipEntry entry;
            while ((entry = zip.getNextEntry()) != null) {
                File out = new File(stage, entry.getName());
                if (!out.getCanonicalPath().startsWith(stage.getCanonicalPath() + "/")) throw new IOException("Invalid runtime path");
                if (entry.isDirectory()) { out.mkdirs(); continue; }
                out.getParentFile().mkdirs();
                try (OutputStream dest = new FileOutputStream(out)) { zip.transferTo(dest); }
            }
        }
        bindLauncher(stage);
        Files.write(new File(stage, ".revision").toPath(), revision.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        deleteTree(runtime);
        if (!stage.renameTo(runtime)) throw new IOException("Cannot activate runtime");
    }

    private void bindLauncher(File runtime) throws Exception {
        File bin = new File(runtime, "bin");
        bin.mkdirs();
        File link = new File(bin, "java");
        Files.deleteIfExists(link.toPath());
        Os.symlink(getApplicationInfo().nativeLibraryDir + "/libmcjava.so", link.getPath());
    }

    private void runPanel() {
        try {
            File runtime = new File(getFilesDir(), "runtime");
            installRuntime(runtime);
            if (stopping) return;
            File data = new File(getFilesDir(), "panel");
            data.mkdirs();
            Files.deleteIfExists(new File(data, ".lock").toPath());
            String nativeDir = getApplicationInfo().nativeLibraryDir;
            ProcessBuilder builder = new ProcessBuilder(nativeDir + "/libmcpanel.so", "--data-dir", data.getPath(),
                "--bind", "127.0.0.1:0", "--allow-unsandboxed-servers");
            builder.directory(getFilesDir()).redirectErrorStream(true)
                .redirectOutput(ProcessBuilder.Redirect.appendTo(new File(getFilesDir(), "host.log")));
            Map<String, String> env = builder.environment();
            env.clear();
            env.put("PATH", "/system/bin");
            env.put("HOME", getFilesDir().getPath());
            env.put("TMPDIR", getCacheDir().getPath());
            env.put("JAVA_HOME", runtime.getPath());
            env.put("MCPANEL_ANDROID_JAVA_HOME", runtime.getPath());
            panel = builder.start();
            if (stopping) panel.destroy();
            long deadline = System.currentTimeMillis() + 60000;
            File lock = new File(data, ".lock");
            while (panel.isAlive() && !stopping && System.currentTimeMillis() < deadline) {
                if (lock.isFile()) {
                    try {
                        JSONObject record = new JSONObject(new String(Files.readAllBytes(lock.toPath()), java.nio.charset.StandardCharsets.UTF_8));
                        String candidate = record.getString("url");
                        java.net.URI uri = new java.net.URI(candidate);
                        if (record.getLong("pid") > 0 && "http".equals(uri.getScheme())
                            && "127.0.0.1".equals(uri.getHost()) && uri.getPort() > 0) {
                            url = candidate;
                            status = "Host running";
                            break;
                        }
                    } catch (Exception ignored) { /* Atomic lock may not be ready yet. */ }
                }
                Thread.sleep(200);
            }
            if (url == null && !stopping) throw new IOException("Panel did not start; see host.log");
            int exit = panel.waitFor();
            status = stopping ? "Host stopped" : "Host exited (" + exit + "); see host.log";
        } catch (Exception error) {
            status = "Host failed: " + error.getMessage();
            android.util.Log.e("MCPANEL", status, error);
        } finally {
            Process child = panel;
            if (child != null && child.isAlive()) {
                child.destroy();
                try { child.waitFor(); } catch (InterruptedException error) { Thread.currentThread().interrupt(); }
            }
            if (stopping) status = "Host stopped";
            url = null;
            stopSelf();
        }
    }

    private static void deleteTree(File root) throws IOException {
        if (!root.exists()) return;
        try (java.util.stream.Stream<Path> paths = Files.walk(root.toPath())) {
            for (Path path : (Iterable<Path>) paths.sorted(Comparator.reverseOrder())::iterator) Files.delete(path);
        }
    }
    @Override public IBinder onBind(Intent intent) { return null; }
    @Override public void onDestroy() {
        active = false;
        stopping = true;
        Process child = panel;
        if (child != null && child.isAlive()) child.destroy();
        if (wake != null && wake.isHeld()) wake.release();
        worker.shutdown();
        super.onDestroy();
    }
}
