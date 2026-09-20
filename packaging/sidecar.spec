from pathlib import Path

from PyInstaller.utils.hooks import collect_all, collect_data_files, copy_metadata

root = Path(SPECPATH).resolve().parent
datas, binaries, hiddenimports = [], [], []
for package in ('spacy', 'presidio_analyzer', 'thinc'):
    package_data, package_binaries, package_imports = collect_all(package)
    datas += package_data
    binaries += package_binaries
    hiddenimports += package_imports
# tldextract's dot-prefixed PSL snapshot is essential on a cold, offline machine.
datas += collect_data_files('tldextract', includes=['.tld_set_snapshot'])
for distribution in ('redactio-sidecar', 'spacy', 'presidio-analyzer', 'tldextract'):
    datas += copy_metadata(distribution, recursive=True)
# Transformers loads model implementations lazily. Torch/transformers hooks
# collect their runtime binaries, source files and dependency metadata.
hiddenimports += [
    'transformers.models.bert.modeling_bert',
    'transformers.models.bert.tokenization_bert_fast',
    'transformers.models.deberta_v2.modeling_deberta_v2',
    'transformers.models.deberta_v2.tokenization_deberta_v2_fast',
    'safetensors.torch',
]
analysis = Analysis(
    [str(root / 'packaging/sidecar-entry.py')],
    pathex=[str(root / 'apps/sidecar/src')], datas=datas, binaries=binaries,
    hiddenimports=hiddenimports,
    excludes=['de_core_news_lg', 'de_core_news_sm', 'pytest', 'mypy'],
)
pyz = PYZ(analysis.pure)
exe = EXE(pyz, analysis.scripts, [], exclude_binaries=True,
          name='redactio-sidecar', console=True, upx=False)
collection = COLLECT(exe, analysis.binaries, analysis.datas,
                     name='redactio-sidecar', upx=False)
