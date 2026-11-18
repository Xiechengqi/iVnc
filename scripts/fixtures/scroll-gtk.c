// Local scroll smoke fixture. Build with pkg-config --cflags --libs gtk+-3.0.
#include <gtk/gtk.h>

static double scroll_x, scroll_y;
static unsigned count;
static GtkAdjustment *vertical;

static gboolean scrolled(GtkWidget *widget, GdkEventScroll *event, gpointer data) {
    double x, y;
    if (gdk_event_get_scroll_deltas((GdkEvent *)event, &x, &y)) {
        scroll_x += x; scroll_y += y; count++;
    }
    return FALSE;
}

static gboolean snapshot(gpointer data) {
    char *json = g_strdup_printf("{\"x\":%.9f,\"y\":%.9f,\"count\":%u,\"top\":%.9f}",
        scroll_x, scroll_y, count, gtk_adjustment_get_value(vertical));
    g_file_set_contents(g_getenv("SCROLL_GTK_STATE"), json, -1, NULL);
    g_free(json);
    return G_SOURCE_CONTINUE;
}

int main(int argc, char **argv) {
    gtk_init(&argc, &argv);
    GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(window), "GTK scroll fixture");
    gtk_window_set_default_size(GTK_WINDOW(window), 960, 720);
    GtkWidget *scroll = gtk_scrolled_window_new(NULL, NULL);
    GtkWidget *view = gtk_text_view_new();
    GString *text = g_string_new(NULL);
    for (unsigned n = 0; n < 10000; n++) g_string_append_printf(text, "Scroll row %u. GTK smooth input fixture.\n", n);
    gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(view)), text->str, text->len);
    g_string_free(text, TRUE);
    gtk_container_add(GTK_CONTAINER(scroll), view);
    gtk_container_add(GTK_CONTAINER(window), scroll);
    gtk_widget_add_events(view, GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK);
    g_signal_connect(view, "scroll-event", G_CALLBACK(scrolled), NULL);
    g_signal_connect(window, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    vertical = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(scroll));
    gtk_widget_show_all(window);
    g_timeout_add(30, snapshot, NULL);
    gtk_main();
    return 0;
}
