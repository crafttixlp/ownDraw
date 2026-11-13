package de.owndraw;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;

/**
 * Minimaler FileProvider (ohne androidx): bietet Dateien aus dem Cache- und Datenordner der App
 * schreibgeschützt als content://de.owndraw.files/{cache|files}/&lt;Pfad&gt; an – für Bilder in der
 * Zwischenablage und zum Öffnen exportierter Dateien in anderen Apps. Nicht exportiert; Zugriff nur
 * über erteilte URI-Rechte (Zwischenablage, FLAG_GRANT_READ_URI_PERMISSION).
 */
public class FileShareProvider extends ContentProvider {
    public static final String AUTHORITY = "de.owndraw.files";

    /** content-URI für eine Datei im Cache- oder Datenordner, sonst null. */
    public static Uri uriFor(Context ctx, File f) {
        try {
            String p = f.getCanonicalPath();
            String[][] roots = {
                {"cache", ctx.getCacheDir().getCanonicalPath()},
                {"files", ctx.getFilesDir().getCanonicalPath()},
            };
            for (String[] r : roots) {
                if (p.startsWith(r[1] + "/")) {
                    return new Uri.Builder().scheme("content").authority(AUTHORITY)
                            .appendPath(r[0]).appendEncodedPath(Uri.encode(p.substring(r[1].length() + 1), "/")).build();
                }
            }
        } catch (IOException e) {
            // nicht teilbar
        }
        return null;
    }

    private File fileFor(Uri uri) throws FileNotFoundException {
        Context ctx = getContext();
        if (ctx == null || uri.getPathSegments().size() < 2) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        String root = uri.getPathSegments().get(0);
        File base = "cache".equals(root) ? ctx.getCacheDir() : ("files".equals(root) ? ctx.getFilesDir() : null);
        if (base == null) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        String rel = uri.getPath().substring(root.length() + 2);
        try {
            File f = new File(base, rel).getCanonicalFile();
            if (!f.getPath().startsWith(base.getCanonicalPath() + "/")) {
                throw new FileNotFoundException("außerhalb: " + uri);
            }
            return f;
        } catch (IOException e) {
            throw new FileNotFoundException(e.toString());
        }
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        if (name == null) {
            return null;
        }
        int dot = name.lastIndexOf('.');
        String ext = dot < 0 ? "" : name.substring(dot + 1).toLowerCase();
        if (ext.equals("owndraw")) {
            return "application/vnd.owndraw+zip";
        }
        if (ext.equals("xopp")) {
            return "application/x-xopp";
        }
        String m = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
        return m == null ? "application/octet-stream" : m;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (mode != null && !mode.equals("r")) {
            throw new SecurityException("nur lesend: " + uri);
        }
        return ParcelFileDescriptor.open(fileFor(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        File f;
        try {
            f = fileFor(uri);
        } catch (FileNotFoundException e) {
            return null;
        }
        String[] cols = projection != null ? projection : new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor c = new MatrixCursor(cols, 1);
        Object[] row = new Object[cols.length];
        for (int i = 0; i < cols.length; i++) {
            if (OpenableColumns.DISPLAY_NAME.equals(cols[i])) {
                row[i] = f.getName();
            } else if (OpenableColumns.SIZE.equals(cols[i])) {
                row[i] = f.length();
            }
        }
        c.addRow(row);
        return c;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("nur lesend");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("nur lesend");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("nur lesend");
    }
}
