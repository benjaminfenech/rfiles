//! Prints the drives the sidebar lists (mount point, kind, source, file system), and which
//! are slow to read. Reads /proc/self/mountinfo and /etc/fstab only: touches no share.
fn main() {
    for d in rf_linux::list_drives() {
        println!("{:<28} {:<10} {:<24} {:<6} slow={}", d.root.display(), format!("{:?}", d.kind), d.source, d.fs, rf_linux::drives::slow_to_read(&d.root));
    }
}
