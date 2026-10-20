class DocExError(Exception):
    """Base exception for document extraction failures."""


class UnsupportedFormatError(DocExError):
    """Raised when a format or conversion is unsupported."""


class MalformedDocumentError(DocExError):
    """Raised when document contents are invalid."""


class UnsafeArchiveError(DocExError):
    """Raised when an OOXML archive violates safety limits."""


class DocumentClosedError(DocExError):
    """Raised when a closed document is accessed."""


class ToolNotFoundError(DocExError):
    """Compatibility exception retained from versions before 0.2.0."""


class ToolExecutionError(DocExError):
    """Compatibility exception retained from versions before 0.2.0."""
