"""Check exact executable/symbol identity without a debugger dependency."""
import hashlib
import json
import math
import pathlib
import re
import struct
import subprocess
import uuid
import zipfile


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def pe_identity(path):
    data = pathlib.Path(path).read_bytes()
    assert data[:2] == b'MZ', 'Expected PE executable'
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    assert data[pe:pe + 4] == b'PE\0\0'
    sections = struct.unpack_from('<H', data, pe + 6)[0]
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    assert struct.unpack_from('<H', data, optional)[0] == 0x20b, 'Expected PE32+'
    debug_rva, debug_size = struct.unpack_from('<II', data, optional + 112 + 6 * 8)
    section_table = optional + optional_size
    for i in range(sections):
        size, address, raw_size, raw = struct.unpack_from('<IIII', data, section_table + i * 40 + 8)
        if address <= debug_rva < address + max(size, raw_size):
            debug = raw + debug_rva - address
            break
    else:
        raise AssertionError('Executable has no mapped debug directory')
    for offset in range(debug, debug + debug_size, 28):
        kind, size, _, pointer = struct.unpack_from('<IIII', data, offset + 12)
        record = data[pointer:pointer + size]
        if kind == 2 and record[:4] == b'RSDS':
            return dict(guid=str(uuid.UUID(bytes_le=record[4:20])),
                        age=struct.unpack_from('<I', record, 20)[0],
                        pdb=record[24:].split(b'\0')[0].decode('utf-8').replace('\\', '/').split('/')[-1])
    raise AssertionError('Executable has no CodeView PDB identity')


def pdb_identity(path):
    # MSF 7.0: superblock -> stream directory -> PDB info stream (stream 1).
    with open(path, 'rb') as source:
        header = source.read(56)
        assert header[:32] == b'Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0', 'Expected MSF 7 PDB'
        block_size, _, blocks, directory_size, _, block_map = struct.unpack_from('<6I', header, 32)
        assert block_size in (512, 1024, 2048, 4096, 8192, 16384, 32768)
        assert blocks * block_size <= pathlib.Path(path).stat().st_size
        directory_blocks = math.ceil(directory_size / block_size)
        assert directory_blocks * 4 <= block_size, 'PDB directory exceeds one block map'
        source.seek(block_map * block_size)
        numbers = struct.unpack('<' + 'I' * directory_blocks, source.read(directory_blocks * 4))
        def read_blocks(numbers, size):
            chunks = []
            for n in numbers:
                assert n < blocks
                source.seek(n * block_size)
                chunks.append(source.read(block_size))
            return b''.join(chunks)[:size]
        directory = read_blocks(numbers, directory_size)
        count = struct.unpack_from('<I', directory)[0]
        assert count > 1
        sizes = struct.unpack_from('<' + 'I' * count, directory, 4)
        pos = 4 + count * 4
        for stream, size in enumerate(sizes):
            n = math.ceil(size / block_size) if size != 0xffffffff else 0
            numbers = struct.unpack_from('<' + 'I' * n, directory, pos)
            pos += n * 4
            if stream == 1:
                info = read_blocks(numbers, size)
                assert len(info) >= 28
                return dict(guid=str(uuid.UUID(bytes_le=info[12:28])), age=struct.unpack_from('<I', info, 8)[0])
    raise AssertionError('PDB info stream missing')


def verify_windows(binary, pdb):
    pe, symbols = pe_identity(binary), pdb_identity(pdb)
    assert pe['guid'] == symbols['guid'] and pe['age'] == symbols['age'], 'PDB does not match executable'
    assert pe['pdb'].lower() == pathlib.Path(pdb).name.lower(), 'Unexpected PDB filename'
    return dict(format='PDB', **symbols)


def macho_identity(path):
    output = subprocess.check_output(['xcrun', 'dwarfdump', '--uuid', str(path)], text=True)
    identities = sorted(line.split()[1:3] for line in output.splitlines() if line.startswith('UUID:'))
    assert identities, 'Mach-O/dSYM UUID missing'
    return identities


def verify_macos(binary, dsym):
    binary_ids = macho_identity(binary)
    assert binary_ids == macho_identity(dsym), 'dSYM does not match executable'
    dwarf = pathlib.Path(dsym) / 'Contents/Resources/DWARF' / pathlib.Path(binary).name
    sections = subprocess.check_output(['xcrun', 'otool', '-l', str(dwarf)], text=True)
    for name in ('__debug_info', '__debug_line'):
        match = re.search(r'sectname ' + name + r'\s+segname __DWARF\s+addr \S+\s+size (0x[0-9a-fA-F]+)', sections)
        assert match and int(match[1], 16) > 0, 'dSYM lacks usable ' + name
    return dict(format='dSYM', uuids=binary_ids)


def zip_symbols(output, files, metadata):
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_STORED) as archive:
        for path, name in files:
            archive.write(path, name)
        archive.writestr('SYMBOLS.json', json.dumps(metadata, indent=2) + '\n')
    output = pathlib.Path(output)
    output.with_suffix(output.suffix + '.sha256').write_text(f'{sha256(output)}  {output.name}\n')


def unpack_dsym(archive, destination):
    with zipfile.ZipFile(archive) as source:
        for member in source.infolist():
            path = pathlib.PurePosixPath(member.filename)
            assert path.parts and path.parts[0] == 'fastrock.dSYM' and not path.is_absolute() and '..' not in path.parts
            source.extract(member, destination)
    return pathlib.Path(destination) / 'fastrock.dSYM'
