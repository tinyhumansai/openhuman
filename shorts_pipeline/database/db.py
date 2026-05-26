from __future__ import annotations

from contextlib import contextmanager
from pathlib import Path

from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

BASE_DIR = Path(__file__).resolve().parents[1]


def build_engine(db_path: str):
    uri = f"sqlite:///{BASE_DIR / db_path}"
    return create_engine(uri, connect_args={"check_same_thread": False}, future=True)


def build_session_factory(engine):
    return sessionmaker(bind=engine, expire_on_commit=False)


@contextmanager
def session_scope(session_factory):
    session = session_factory()
    try:
        yield session
        session.commit()
    except Exception:
        session.rollback()
        raise
    finally:
        session.close()
