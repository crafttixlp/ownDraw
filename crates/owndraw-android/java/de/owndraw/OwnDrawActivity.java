package de.owndraw;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;
import android.view.DragAndDropPermissions;
import android.view.DragEvent;
import android.view.View;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.util.ArrayList;

/**
 * Hülle um die NativeActivity von OwnDraw (Rust/egui in libowndraw.so).
 *
 * Leitet an die native Seite weiter, was nur Java mitbekommt: Ergebnisse der SAF-Dialoge
 * (onActivityResult), onPause, Start-/neue Intents („Öffnen mit“, „Teilen“) und Drag &amp; Drop.
 * Dazu Hilfsmethoden, die Rust per JNI aufruft: Zwischenablage (clipSet/clipGet) und openUrl.
 */
public class OwnDrawActivity extends NativeActivity {
    private static final String TAG = "OwnDraw";

    static {
        // NativeActivity lädt die Bibliothek selbst per dlopen; für die JNI-Methoden unten muss sie
        // zusätzlich über den ClassLoader registriert sein (gleiche Datei, gleicher Handle).
        System.loadLibrary("owndraw");
    }

    private native void nativeOnPicked(int tag, String uri);
    private native void nativeOnPause();
    private native void nativeOnOpen(String uri);
    private native void nativeOnDrop(String uri, String text, String mime, float x, float y);
    private native void nativeOnDragHover(boolean hover);

    /** Rechte an fallen gelassenen URIs; gelten, bis sie freigegeben werden oder die Activity endet. */
    private final ArrayList<DragAndDropPermissions> dropPermissions = new ArrayList<>();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().getDecorView().setOnDragListener(new View.OnDragListener() {
            @Override
            public boolean onDrag(View v, DragEvent e) {
                return handleDrag(e);
            }
        });
        // Nach Neuerstellung (Drehen o. Ä.) den Start-Intent nicht erneut öffnen.
        if (savedInstanceState == null) {
            handleIntent(getIntent());
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    @Override
    protected void onPause() {
        // Vor super.onPause(): die App speichert, solange sie noch läuft (wartet höchstens kurz).
        nativeOnPause();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        for (DragAndDropPermissions p : dropPermissions) {
            p.release();
        }
        dropPermissions.clear();
        super.onDestroy();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        Uri uri = (resultCode == RESULT_OK && data != null) ? data.getData() : null;
        if (uri != null) {
            persist(uri, data.getFlags());
        }
        nativeOnPicked(requestCode, uri == null ? null : uri.toString());
    }

    /** Dauerhafte Rechte (übersteht Neustart), soweit der Anbieter sie gewährt. */
    private void persist(Uri uri, int flags) {
        int f = flags & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        if (f == 0 || (flags & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) == 0) {
            return;
        }
        try {
            getContentResolver().takePersistableUriPermission(uri, f);
        } catch (SecurityException e) {
            Log.w(TAG, "takePersistableUriPermission: " + e);
        }
    }

    /** VIEW („Öffnen mit“) und SEND („Teilen an OwnDraw“). */
    private void handleIntent(Intent intent) {
        if (intent == null) {
            return;
        }
        String action = intent.getAction();
        Uri uri = null;
        if (Intent.ACTION_VIEW.equals(action)) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(action)) {
            uri = streamExtra(intent);
        }
        if (uri != null) {
            persist(uri, intent.getFlags());
            nativeOnOpen(uri.toString());
        }
    }

    @SuppressWarnings("deprecation")
    private static Uri streamExtra(Intent intent) {
        Object o = intent.getParcelableExtra(Intent.EXTRA_STREAM);
        if (o instanceof Uri) {
            return (Uri) o;
        }
        ClipData c = intent.getClipData();
        if (c != null && c.getItemCount() > 0) {
            return c.getItemAt(0).getUri();
        }
        return null;
    }

    // ------------------------------------------------------------- Drag & Drop

    private boolean handleDrag(DragEvent e) {
        switch (e.getAction()) {
            case DragEvent.ACTION_DRAG_STARTED:
                return true;
            case DragEvent.ACTION_DRAG_ENTERED:
                nativeOnDragHover(true);
                return true;
            case DragEvent.ACTION_DRAG_EXITED:
            case DragEvent.ACTION_DRAG_ENDED:
                nativeOnDragHover(false);
                return true;
            case DragEvent.ACTION_DROP:
                nativeOnDragHover(false);
                DragAndDropPermissions p = requestDragAndDropPermissions(e);
                if (p != null) {
                    dropPermissions.add(p);
                }
                ClipData clip = e.getClipData();
                if (clip == null) {
                    return false;
                }
                ClipDescription desc = e.getClipDescription();
                String mime = (desc != null && desc.getMimeTypeCount() > 0) ? desc.getMimeType(0) : null;
                for (int i = 0; i < clip.getItemCount(); i++) {
                    ClipData.Item it = clip.getItemAt(i);
                    Uri u = it.getUri();
                    CharSequence t = it.getText();
                    String m = mime;
                    if (u != null) {
                        String real = getContentResolver().getType(u);
                        if (real != null) {
                            m = real;
                        }
                    }
                    nativeOnDrop(u == null ? null : u.toString(), t == null ? null : t.toString(), m, e.getX(), e.getY());
                }
                return true;
            default:
                return true;
        }
    }

    // ------------------------------------------------------------- Aufrufe aus Rust (JNI)

    /**
     * Setzt die Zwischenablage. {@code pngPath}: Bild im Cache-Ordner (wird per FileShareProvider als
     * content://-URI angeboten), {@code text}: Text bzw. interne Items mit Präfix "owndraw-items:".
     * Beides darf null sein; Bild steht vorn, damit andere Apps es als erstes Element finden.
     */
    public void clipSet(final String text, final String pngPath) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                    ClipData clip = null;
                    Uri img = pngPath == null ? null : FileShareProvider.uriFor(OwnDrawActivity.this, new File(pngPath));
                    if (img != null) {
                        clip = ClipData.newUri(getContentResolver(), "OwnDraw", img);
                        if (text != null) {
                            clip.addItem(getContentResolver(), new ClipData.Item(text));
                        }
                    } else if (text != null) {
                        clip = ClipData.newPlainText("OwnDraw", text);
                    }
                    if (clip != null) {
                        cm.setPrimaryClip(clip);
                    }
                } catch (RuntimeException e) {
                    Log.w(TAG, "clipSet: " + e);
                }
            }
        });
    }

    /** Inhalt der Zwischenablage: "t" + Text bzw. "u" + MIME + "\n" + URI je Eintrag. */
    public String[] clipGet() {
        ArrayList<String> out = new ArrayList<>();
        try {
            ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
            ClipData clip = cm.getPrimaryClip();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    ClipData.Item it = clip.getItemAt(i);
                    Uri u = it.getUri();
                    if (u != null) {
                        String m = getContentResolver().getType(u);
                        out.add("u" + (m == null ? "" : m) + "\n" + u);
                    }
                    CharSequence t = it.getText();
                    if (t != null) {
                        out.add("t" + t);
                    }
                }
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "clipGet: " + e);
        }
        return out.toArray(new String[0]);
    }

    /** Öffnet eine URL im Browser bzw. eine Datei aus dem App-Speicher (über FileShareProvider). */
    public void openUrl(String url) {
        try {
            Intent i = new Intent(Intent.ACTION_VIEW);
            String path = url.startsWith("file://") ? Uri.parse(url).getPath() : (url.startsWith("/") ? url : null);
            if (path != null) {
                File f = new File(path);
                Uri u = FileShareProvider.uriFor(this, f);
                if (u == null) {
                    Log.w(TAG, "openUrl: Datei liegt nicht im App-Speicher: " + path);
                    return;
                }
                String ext = MimeTypeMap.getFileExtensionFromUrl(f.getName());
                String mime = ext == null ? null : MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase());
                i.setDataAndType(u, mime == null ? "*/*" : mime);
                i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } else {
                i.setData(Uri.parse(url));
            }
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(i);
        } catch (ActivityNotFoundException | SecurityException e) {
            Log.w(TAG, "openUrl: " + e);
        }
    }
}
