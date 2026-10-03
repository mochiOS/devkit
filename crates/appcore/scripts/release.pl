#!/usr/bin/env perl

use strict;
use warnings;

use Cwd qw(abs_path);
use Digest::SHA ();
use File::Basename qw(basename dirname);
use File::Copy qw(copy);
use File::Path qw(make_path);
use File::Spec;
use File::Temp qw(tempdir);
use FindBin qw($Bin);
use Getopt::Long qw(GetOptions);

my $appcore_root = abs_path(File::Spec->catdir($Bin, '..'));
my $workspace_root = abs_path(File::Spec->catdir($appcore_root, '..', '..'));
my $target;
my $arch;
my $sdk_version;
my $mochios_root;

GetOptions(
    'sdk-version=s' => \$sdk_version,
    'mochios-root=s' => \$mochios_root,
    'target=s'      => \$target,
    'arch=s'        => \$arch,
) or die usage();

@ARGV == 0 or die usage();

my $version = manifest_version(File::Spec->catfile($appcore_root, 'Cargo.toml'));
my $kome_version = manifest_version(File::Spec->catfile($appcore_root, 'Kome.toml'));
$version eq $kome_version
    or die "AppCore version mismatch: Cargo.toml has $version, Kome.toml has $kome_version\n";

$mochios_root //= $ENV{MOCHIOS_ROOT};
$mochios_root //= File::Spec->catdir($workspace_root, '..', '..', '..', 'mochiOS');
$mochios_root = abs_path($mochios_root)
    // die "mochiOS root was not found; pass --mochios-root <path>\n";
require_directory(File::Spec->catdir($mochios_root, 'user'));
require_directory(File::Spec->catdir($mochios_root, 'libraries'));
require_file(File::Spec->catfile($mochios_root, 'tools', 'devkit', 'Cargo.lock'));

$arch //= architecture_name($target);
validate_fragment('architecture', $arch);
validate_fragment('version', $version);
if (defined $sdk_version) {
    validate_fragment('SDK version', $sdk_version);
} else {
    $sdk_version = $version;
}

build_appcore($target, $mochios_root);

my $binary_directory = defined $target
    ? File::Spec->catdir($workspace_root, 'target', $target, 'release')
    : File::Spec->catdir($workspace_root, 'target', 'release');
my $shared_library = File::Spec->catfile($binary_directory, 'libappcore.so');
my $static_library = File::Spec->catfile($binary_directory, 'libappcore.a');
require_file($shared_library);
require_file($static_library);

my $stage = tempdir('appcore-release-XXXXXX', TMPDIR => 1, CLEANUP => 1);
stage_file('Kome.toml', $stage);
stage_kome_sources($stage);
stage_file(File::Spec->catfile('include', 'mochios.h'), $stage);
stage_file(File::Spec->catfile('include', 'mochios_abi.h'), $stage);
stage_file('README.md', $stage);
copy_file(File::Spec->catfile($workspace_root, 'LICENSE'), File::Spec->catfile($stage, 'LICENSE'));
copy_file($shared_library, File::Spec->catfile($stage, basename($shared_library)));
copy_file($static_library, File::Spec->catfile($stage, basename($static_library)));

my @entries = qw(Kome.toml LICENSE README.md lib include libappcore.a libappcore.so);
my $output_directory = File::Spec->catdir($workspace_root, 'target', 'release');
make_path($output_directory);
my $filename = "$arch-appcore-$sdk_version.tar.zst";
my $archive = File::Spec->catfile($output_directory, $filename);
my $tar = File::Spec->catfile($stage, "$arch-appcore-$sdk_version.tar");

run(
    'tar',
    '--sort=name',
    '--mtime=@' . source_date_epoch(),
    '--owner=0',
    '--group=0',
    '--numeric-owner',
    '-C', $stage,
    '-cf', $tar,
    @entries,
);
run('zstd', '-q', '-19', '-f', $tar, '-o', $archive);
write_checksum($archive, File::Spec->catfile($output_directory, 'SHA256SUMS'));

print "$archive\n";

sub usage {
    return "usage: crates/appcore/scripts/release.pl [--sdk-version <version>] [--mochios-root <path>] [--target <rust-target>] [--arch <artifact-arch>]\n";
}

sub manifest_version {
    my ($path) = @_;
    open my $file, '<', $path or die "failed to read $path: $!\n";
    my $in_package = 0;
    while (my $line = <$file>) {
        if ($line =~ /^\[package\]\s*$/) {
            $in_package = 1;
            next;
        }
        last if $in_package && $line =~ /^\[/;
        if ($in_package && $line =~ /^version\s*=\s*"([^"]+)"\s*$/) {
            close $file or die "failed to close $path: $!\n";
            return $1;
        }
    }
    close $file or die "failed to close $path: $!\n";
    die "package version was not found in $path\n";
}

sub architecture_name {
    my ($target) = @_;
    return (split /-/, $target, 2)[0] if defined $target;

    open my $rustc, '-|', 'rustc', '-vV' or die "failed to start rustc: $!\n";
    my $host;
    while (my $line = <$rustc>) {
        $host = $1 if $line =~ /^host:\s+([^\s]+)/;
    }
    close $rustc or die "rustc -vV failed\n";
    defined $host or die "rustc did not report a host architecture\n";
    return (split /-/, $host, 2)[0];
}

sub validate_fragment {
    my ($description, $value) = @_;
    $value =~ /\A[A-Za-z0-9][A-Za-z0-9._+-]*\z/
        or die "invalid $description: $value\n";
}

sub build_appcore {
    my ($target, $mochios_root) = @_;
    my $build_root = tempdir('appcore-build-XXXXXX', TMPDIR => 1, CLEANUP => 1);
    my $build_package = File::Spec->catdir($build_root, 'tools', 'devkit', 'crates', 'appcore');
    make_path($build_package);
    copy_file(
        File::Spec->catfile($appcore_root, 'Cargo.toml'),
        File::Spec->catfile($build_package, 'Cargo.toml'),
    );
    copy_file(
        File::Spec->catfile($mochios_root, 'tools', 'devkit', 'Cargo.lock'),
        File::Spec->catfile($build_package, 'Cargo.lock'),
    );
    link_directory(
        File::Spec->catdir($appcore_root, 'src'),
        File::Spec->catdir($build_package, 'src'),
    );
    link_directory(
        File::Spec->catdir($mochios_root, 'user'),
        File::Spec->catdir($build_root, 'user'),
    );
    link_directory(
        File::Spec->catdir($mochios_root, 'libraries'),
        File::Spec->catdir($build_root, 'libraries'),
    );

    run(
        'cargo', 'generate-lockfile',
        '--manifest-path', File::Spec->catfile($build_package, 'Cargo.toml'),
        '--offline',
    );

    my @command = (
        'cargo', 'build',
        '--manifest-path', File::Spec->catfile($build_package, 'Cargo.toml'),
        '--target-dir', File::Spec->catdir($workspace_root, 'target'),
        '--release', '--locked',
    );
    push @command, '--target', $target if defined $target;
    run(@command);
}

sub link_directory {
    my ($source, $destination) = @_;
    require_directory($source);
    make_path(dirname($destination));
    symlink $source, $destination
        or die "failed to link $source to $destination: $!\n";
}

sub stage_file {
    my ($relative, $stage) = @_;
    copy_file(
        File::Spec->catfile($appcore_root, $relative),
        File::Spec->catfile($stage, $relative),
    );
}

sub stage_kome_sources {
    my ($stage) = @_;
    my $source_directory = File::Spec->catdir($appcore_root, 'lib', 'src');
    opendir my $directory, $source_directory
        or die "failed to read $source_directory: $!\n";
    my @sources = sort grep { /\.kome\z/ && -f File::Spec->catfile($source_directory, $_) }
        readdir $directory;
    closedir $directory or die "failed to close $source_directory: $!\n";
    @sources or die "no Kome library sources were found in $source_directory\n";
    for my $source (@sources) {
        stage_file(File::Spec->catfile('lib', 'src', $source), $stage);
    }
}

sub copy_file {
    my ($source, $destination) = @_;
    require_file($source);
    make_path(dirname($destination));
    copy($source, $destination)
        or die "failed to copy $source to $destination: $!\n";
}

sub require_file {
    my ($path) = @_;
    -f $path or die "required release file was not found: $path\n";
}

sub require_directory {
    my ($path) = @_;
    -d $path or die "required directory was not found: $path\n";
}

sub source_date_epoch {
    return $ENV{SOURCE_DATE_EPOCH}
        if defined $ENV{SOURCE_DATE_EPOCH} && $ENV{SOURCE_DATE_EPOCH} =~ /\A\d+\z/;

    open my $git, '-|', 'git', '-C', $workspace_root, 'log', '-1', '--format=%ct'
        or die "failed to start git: $!\n";
    my $epoch = <$git>;
    close $git or die "git log failed\n";
    chomp $epoch;
    $epoch =~ /\A\d+\z/ or die "git returned an invalid source timestamp\n";
    return $epoch;
}

sub write_checksum {
    my ($archive, $path) = @_;
    open my $input, '<', $archive or die "failed to read $archive: $!\n";
    binmode $input;
    my $digest = Digest::SHA->new(256)->addfile($input)->hexdigest;
    close $input or die "failed to close $archive: $!\n";

    my $temporary = "$path.tmp.$$";
    open my $output, '>', $temporary or die "failed to write $temporary: $!\n";
    print {$output} "$digest  " . basename($archive) . "\n";
    close $output or die "failed to close $temporary: $!\n";
    rename $temporary, $path or die "failed to replace $path: $!\n";
}

sub run {
    my (@command) = @_;
    print '+ ' . join(' ', map { shell_quote($_) } @command) . "\n";
    system @command;
    if ($? == -1) {
        die "failed to execute $command[0]: $!\n";
    }
    if ($? & 127) {
        die "$command[0] terminated by signal " . ($? & 127) . "\n";
    }
    my $exit = $? >> 8;
    $exit == 0 or die "$command[0] exited with status $exit\n";
}

sub shell_quote {
    my ($value) = @_;
    return $value if $value =~ /\A[-A-Za-z0-9_.,\/:=+@]+\z/;
    $value =~ s/'/'"'"'/g;
    return "'$value'";
}
